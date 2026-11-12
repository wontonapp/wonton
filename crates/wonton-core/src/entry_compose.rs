//! TLS 解析在隔离的 HAProxy 容器，连接器只转密文，核心仅接受入口专用网络的 HTTP。

use std::os::unix::fs::{MetadataExt, PermissionsExt, lchown};
use std::path::Path;

use anyhow::Context;
use serde_json::{Value, json};

use crate::entry_settings::EntrySettings;

pub const HAPROXY_IMAGE: &str =
    "haproxy:3.2.8-alpine@sha256:8c6b55d891584afded3ab1b91063a703ecc8ff2481302b691773648ffa806226";

pub fn render(
    base: &str,
    settings: Option<&EntrySettings>,
    install_dir: &Path,
) -> anyhow::Result<String> {
    let mut compose: Value = serde_saphyr::from_str(base)?;
    compose["services"]["tuwunel"]["volumes"] = json!([
        "tuwunel:/var/lib/tuwunel",
        "${WONTON_INSTALL_DIR}/secrets/tuwunel:/etc/tuwunel:ro"
    ]);
    let Some(settings) = settings else {
        return Ok(serde_json::to_string_pretty(&compose)?);
    };
    let meta = std::fs::metadata(install_dir.join("entry/certs/site.pem"))?;
    let uid = meta.uid();
    let gid = meta.gid();
    let user = format!("{uid}:{gid}");
    compose["services"]["core"]["volumes"]
        .as_array_mut()
        .context("core volumes 不是列表")?
        .push(json!(
            "${WONTON_INSTALL_DIR}/entry:${WONTON_INSTALL_DIR}/entry"
        ));
    compose["services"]["core"]["volumes"]
        .as_array_mut()
        .context("core volumes 不是列表")?
        .push(json!(
            "${WONTON_INSTALL_DIR}/entry-input:${WONTON_INSTALL_DIR}/entry-input:ro"
        ));
    compose["services"]["core"]["networks"]["clients"] =
        json!({"aliases":["entry-gateway.wonton.internal"]});
    // 配了解析器时核心和 lego 都用它：入口的 ACME 目录、acme-dns 和 CAA 都靠这些名字。
    let resolvers = crate::entry_settings::resolver_ips(&settings.acme_resolvers)?;
    if !resolvers.is_empty() {
        // Docker 的 127.0.0.11 在前：容器名、插件别名只有它认识；它认不出的名字转发给配置的解析器。
        // 反过来先问真实解析器时，它们在区外名字上回 NXDOMAIN，glibc 不再回退，内部名就解析不到。
        let mut servers = vec!["127.0.0.11".to_owned()];
        servers.extend(resolvers);
        compose["services"]["core"]["dns"] = json!(servers);
    }
    compose["services"]["core"]["environment"]["WONTON_ENTRY_LISTEN"] =
        json!("entry-gateway.wonton.internal:8484");
    compose["services"]["core"]["environment"]["WONTON_MATRIX_FEDERATION_ADDR"] =
        json!("entry.wonton.internal:8449");
    // 发布的 UDP 端口由 Docker 按容器端点做 DNAT，命中的接口不确定；隧道绑所有地址，哪个端点都收得到。
    compose["services"]["core"]["environment"]["WONTON_TUNNEL_BIND"] = json!("0.0.0.0");
    // 客户端网关不能在入口接口上接受连接，防止已攻破入口读取首次配对码。
    compose["services"]["core"]["environment"]["WONTON_EXCLUDE_LISTEN"] =
        json!("entry-gateway.wonton.internal");
    for network in ["clients", "edge"] {
        compose["networks"][network] = json!({"internal":true,"driver_opts":{
            "com.docker.network.bridge.gateway_mode_ipv4":"isolated",
            "com.docker.network.bridge.gateway_mode_ipv6":"isolated"
        }});
    }
    compose["networks"]["relay"] = json!({});
    let entry = json!({
        "image":HAPROXY_IMAGE,"user":user,"read_only":true,"cap_drop":["ALL"],
        "security_opt":["no-new-privileges:true"],"restart":"unless-stopped",
        "command":["haproxy","-W","-db","-f","/config/haproxy.cfg"],
        "volumes":["${WONTON_INSTALL_DIR}/entry/haproxy.cfg:/config/haproxy.cfg:ro","${WONTON_INSTALL_DIR}/entry/certs:/certs:ro"],
        "networks":{"edge":{"aliases":["entry-edge.wonton.internal"]},"clients":{"aliases":["entry.wonton.internal"]}},
        "environment":{"HTTP_PROXY":"","HTTPS_PROXY":"","http_proxy":"","https_proxy":""},
        "logging":{"driver":"json-file","options":{"max-size":"10m","max-file":"3"}}
    });
    compose["services"]["entry"] = entry;
    if settings.registration.is_some() {
        compose["services"]["connector"] = json!({
            "image":crate::entry_runtime::FRPC_IMAGE,"user":user,"read_only":true,"cap_drop":["ALL"],
            "security_opt":["no-new-privileges:true"],"restart":"unless-stopped",
            "command":["-c","/config/frpc.json"],
            "volumes":["${WONTON_INSTALL_DIR}/entry/frpc.json:/config/frpc.json:ro","${WONTON_INSTALL_DIR}/entry/relay-ca.pem:/config/relay-ca.pem:ro"],
            "environment":{"HTTP_PROXY":"","HTTPS_PROXY":"","http_proxy":"","https_proxy":""},
            "networks":{"edge":{"aliases":["connector.wonton.internal"]},"relay":null},
            "logging":{"driver":"json-file","options":{"max-size":"10m","max-file":"3"}}
        });
    } else {
        compose["services"]["connector"] = json!({
            "image":HAPROXY_IMAGE,"user":user,"read_only":true,"cap_drop":["ALL"],
            "security_opt":["no-new-privileges:true"],"restart":"unless-stopped",
            "command":["haproxy","-W","-db","-f","/config/connector.cfg"],
            "volumes":["${WONTON_INSTALL_DIR}/entry/connector.cfg:/config/connector.cfg:ro"],
            "ports":["${WONTON_ENTRY_BIND:-127.0.0.1}:${WONTON_ENTRY_PORT:-8443}:8443"],
            "networks":{"edge":{"aliases":["connector.wonton.internal"]},"relay":null}
        });
    }
    Ok(serde_json::to_string_pretty(&compose)?)
}

pub fn write(install_dir: &Path, settings: &EntrySettings) -> anyhow::Result<()> {
    let directory = install_dir.join("entry");
    std::fs::create_dir_all(&directory)?;
    let bind =
        "bind entry-edge.wonton.internal:8443 accept-proxy ssl crt /certs/site.pem strict-sni";
    let domain = &settings.domain;
    let config = format!(
        "global\n  log stdout format raw local0\n  maxconn 1024\n\ndefaults\n  mode http\n  timeout connect 5s\n  timeout client 60s\n  timeout server 60s\n  option http-restrict-req-hdr-names delete\n\nresolvers docker\n  nameserver docker 127.0.0.11:53\n  resolve_retries 1\n  timeout resolve 1s\n  timeout retry 1s\n  hold valid 10s\n\nfrontend public\n  {bind}\n  acl correct_sni ssl_fc_sni -i {domain}\n  http-request deny deny_status 421 unless correct_sni\n  http-request deny deny_status 421 unless {{ hdr(host),lower -i {domain} {domain}:443 }}\n  http-request del-header Forwarded\n  http-request del-header X-Forwarded-For\n  http-request del-header X-Forwarded-Host\n  http-request del-header X-Forwarded-Proto\n  http-request del-header X-Real-IP\n  http-request del-header X-Wonton-Entry\n  http-request set-header X-Real-IP %[src]\n  http-request set-header X-Wonton-Entry public\n  default_backend gateway\n\nfrontend paired\n  bind entry.wonton.internal:8449 ssl crt /certs/site.pem strict-sni\n  acl correct_sni ssl_fc_sni -i {domain}\n  http-request deny deny_status 421 unless correct_sni\n  http-request deny deny_status 421 unless {{ hdr(host),lower -i {domain} {domain}:443 {domain}:8448 }}\n  http-request set-header Host {domain}\n  default_backend gateway\n\nbackend gateway\n  server core entry-gateway.wonton.internal:8484 resolvers docker init-addr libc,none\n"
    );
    crate::install::write_private(&directory.join("haproxy.cfg"), &config)?;
    if let Some(home) = &settings.registration {
        let trusted_ca = settings
            .relay_ca
            .as_ref()
            .context("中继连接必须配置 WONTON_FRP_CA_FILE（校验 frps 证书）")?;
        let ca = std::fs::read_to_string(trusted_ca)?;
        crate::install::write_private(&directory.join("relay-ca.pem"), &ca)?;
        let frpc = json!({
            "serverAddr":home.relay.host,"serverPort":home.relay.port,"user":home.home,
            "metadatas":{"token":home.frp_token},"loginFailExit":true,
            "transport":{"tls":{"enable":true,"disableCustomTLSFirstByte":false,"serverName":home.relay.server_name,"trustedCaFile":"/config/relay-ca.pem"},"heartbeatInterval":30,"heartbeatTimeout":90},
            "proxies":[{"name":"matrix","type":"https","localIP":"entry-edge.wonton.internal","localPort":8443,"customDomains":[home.domain],"transport":{"proxyProtocolVersion":"v2"}}]
        });
        crate::install::write_private(
            &directory.join("frpc.json"),
            &serde_json::to_string_pretty(&frpc)?,
        )?;
    } else {
        crate::install::write_private(
            &directory.join("connector.cfg"),
            "global\n  maxconn 1024\ndefaults\n  mode tcp\n  timeout connect 5s\n  timeout client 60s\n  timeout server 60s\nfrontend public\n  bind :8443\n  default_backend entry\nbackend entry\n  server entry entry-edge.wonton.internal:8443 send-proxy-v2\n",
        )?;
    }
    prepare_permissions(install_dir)?;
    Ok(())
}

pub fn prepare_permissions(dir: &Path) -> anyhow::Result<()> {
    for name in [
        "haproxy.cfg",
        "connector.cfg",
        "frpc.json",
        "relay-ca.pem",
        "certs",
        "certs/site.pem",
        "certs/site.key",
    ] {
        let path = dir.join("entry").join(name);
        let Ok(meta) = std::fs::metadata(&path) else {
            continue;
        };
        if meta.uid() == 0 {
            lchown(&path, Some(99), Some(99))?;
        }
        std::fs::set_permissions(
            &path,
            std::fs::Permissions::from_mode(if meta.is_dir() { 0o700 } else { 0o600 }),
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entry_settings::EntrySettings;
    use serde_json::Value;

    fn settings() -> EntrySettings {
        EntrySettings {
            domain: "home.example".to_owned(),
            service_url: None,
            service_ca: None,
            relay_ca: None,
            registration: None,
            acme_directory: None,
            acme_ca: None,
            acme_resolvers: Vec::new(),
            acme_email: None,
            acme_accept_terms: false,
            acme_network: "bridge".to_owned(),
            dns_provider: None,
            dns_env_file: None,
            external_certificate: false,
        }
    }

    fn tuwenel(rendered: &str) -> Value {
        serde_json::from_str::<Value>(rendered).unwrap()["services"]["tuwunel"].clone()
    }

    /// 内部名（`entry.wonton.internal`、插件别名）只有 Docker 的 127.0.0.11 认识；真实解析器
    /// 对区外名字回 NXDOMAIN 时 glibc 不再回退，所以内置 DNS 必须排在最前。
    #[test]
    fn core_resolvers_ask_the_embedded_dns_before_the_configured_ones() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("entry/certs")).unwrap();
        std::fs::write(dir.path().join("entry/certs/site.pem"), "pem").unwrap();
        let mut settings = settings();
        settings.acme_resolvers =
            crate::entry_settings::parse_resolvers("192.0.2.53:53, 198.51.100.53:53").unwrap();
        let compose: Value = serde_json::from_str(
            &render(crate::install::COMPOSE, Some(&settings), dir.path()).unwrap(),
        )
        .unwrap();
        assert_eq!(
            compose["services"]["core"]["dns"],
            json!(["127.0.0.11", "192.0.2.53", "198.51.100.53"])
        );
    }

    /// 联邦容器只该看见自己的文件：`secrets/` 根上的 `matrix-jwt.pem`（能签登录令牌）和
    /// `hydra-secret`/`hydra.env`（`SECRETS_SYSTEM`）都不该挂进去。
    #[test]
    fn only_tuwunel_files_are_mounted_into_the_federation_container() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("entry/certs")).unwrap();
        std::fs::write(dir.path().join("entry/certs/site.pem"), "pem").unwrap();
        for rendered in [
            render(crate::install::COMPOSE, None, dir.path()).unwrap(),
            render(crate::install::COMPOSE, Some(&settings()), dir.path()).unwrap(),
        ] {
            let tuwenel = tuwenel(&rendered);
            let sources: Vec<String> = tuwenel["volumes"]
                .as_array()
                .unwrap()
                .iter()
                .map(|volume| {
                    let volume = volume.as_str().unwrap();
                    volume.split(':').next().unwrap().to_owned()
                })
                .collect();
            assert_eq!(
                sources,
                ["tuwunel", "${WONTON_INSTALL_DIR}/secrets/tuwunel"],
                "{sources:?}"
            );
            for source in &sources {
                assert!(!source.ends_with("/secrets"), "{source} 是 secrets 整目录");
                assert!(!source.contains("matrix-jwt"), "{source} 是核心私钥");
                assert!(!source.contains("hydra"), "{source} 是 Hydra 的主密钥");
            }
            assert_eq!(
                tuwenel["command"],
                json!([
                    "-c",
                    "/etc/tuwunel/tuwunel.toml",
                    "-c",
                    "/etc/tuwunel/wonton-fed.toml"
                ]),
                "第二份配置要落在挂进来的目录里"
            );
        }
    }

    /// 裸 IP（不带端口）也要渲染进核心的 `dns`，不能被静默丢掉（#226）。
    #[test]
    fn bare_resolver_ips_render_into_core_dns() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("entry/certs")).unwrap();
        std::fs::write(dir.path().join("entry/certs/site.pem"), "pem").unwrap();
        let mut settings = settings();
        settings.acme_resolvers = vec![
            "192.0.2.53".to_owned(),
            "198.51.100.53:5353".to_owned(),
            "2001:db8::53".to_owned(),
            "[2001:db8::54]:53".to_owned(),
        ];
        let compose: Value = serde_json::from_str(
            &render(crate::install::COMPOSE, Some(&settings), dir.path()).unwrap(),
        )
        .unwrap();
        assert_eq!(
            compose["services"]["core"]["dns"],
            json!([
                "127.0.0.11",
                "192.0.2.53",
                "198.51.100.53",
                "2001:db8::53",
                "2001:db8::54"
            ])
        );
    }
}
