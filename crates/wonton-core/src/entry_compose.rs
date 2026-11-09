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
        "${WONTON_INSTALL_DIR}/secrets:/etc/tuwunel:ro",
        "${WONTON_INSTALL_DIR}/fed/config.toml:/etc/wonton-fed.toml:ro"
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
    let resolvers: Vec<_> = settings
        .acme_resolvers
        .iter()
        .filter_map(|server| {
            server
                .parse::<std::net::SocketAddr>()
                .ok()
                .map(|address| address.ip().to_string())
        })
        .collect();
    if !resolvers.is_empty() {
        // 配置的解析器在前：Docker 的 127.0.0.11 对认不出的名字回 NXDOMAIN，会截断回退；
        // 测试解析器对区外名字回 REFUSED，才落到 127.0.0.11 去解析内部别名。
        let mut servers = resolvers;
        servers.push("127.0.0.11".to_owned());
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
