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
    _install_dir: &Path,
) -> anyhow::Result<String> {
    let mut compose: Value = serde_saphyr::from_str(base)?;
    compose["services"]["tuwunel"]["volumes"] = json!([
        "tuwunel:/var/lib/tuwunel",
        "${WONTON_INSTALL_DIR}/secrets/tuwunel:/etc/tuwunel:ro"
    ]);
    // 运行时签发：核心经 PUT 收设置后写 `entry/`（CSR、ACME 账号、dns.env），lego 容器按宿主路径绑挂
    // `entry/acme` 和 `entry/requests/`。核心容器里必须看得到同一个宿主目录，不然签发的产物只落进容器
    // 可写层，lego 拿不到 CSR；CA 文件在 `entry-input/` 里，核心也要读。挂载本身不依赖 settings，
    // 提前返回会把这一步整个跳掉。
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
    // 网络和核心在这几张网络上的绑定不管有没有入口设置都渲染：启动时就能定下 8484 绑在哪、主网关排除哪个地址；
    // 容器本身由核心按设置建（`entry_containers`）。
    compose["services"]["core"]["networks"]["clients"] =
        json!({"aliases":["entry-gateway.wonton.internal"]});
    compose["services"]["core"]["environment"]["WONTON_ENTRY_LISTEN"] =
        json!("entry-gateway.wonton.internal:8484");
    // 隧道只绑核心在 core 网络上的地址（ADR-0021）：被攻破的入口容器在 clients 网络上发到隧道端口的包
    // 落不到套接字。绑的是别名，由 Docker 的内嵌 DNS 在启动时解析；发布的 UDP 端口经 Docker DNAT 落在
    // core 网络的端点上（tests/docker_tunnel_bind.rs 断言），局域网和公网来的包照样到得了。
    compose["services"]["core"]["networks"]["core"]["aliases"]
        .as_array_mut()
        .context("core 网络的 aliases 不是列表")?
        .push(json!("tunnel.wonton.internal"));
    compose["services"]["core"]["environment"]["WONTON_TUNNEL_BIND"] =
        json!("tunnel.wonton.internal");
    // 客户端网关不能在入口接口上接受连接，防止已攻破入口读取首次配对码。
    compose["services"]["core"]["environment"]["WONTON_EXCLUDE_LISTEN"] =
        json!("entry-gateway.wonton.internal");
    compose["networks"]["clients"] = json!({"internal":true,"driver_opts":{
        "com.docker.network.bridge.gateway_mode_ipv4":"isolated",
        "com.docker.network.bridge.gateway_mode_ipv6":"isolated"
    }});
    // 连接器用固定地址：入口认对端在连接层，那里只认 IP（见 `entry_haproxy`）。
    compose["networks"]["edge"] = json!({"internal":true,
    "ipam":{"config":[{"subnet":crate::entry_haproxy::EDGE_SUBNET}]},
    "driver_opts":{
        "com.docker.network.bridge.gateway_mode_ipv4":"isolated",
        "com.docker.network.bridge.gateway_mode_ipv6":"isolated"
    }});
    // 连接器只经这张普通桥接出网。
    compose["networks"]["relay"] = json!({});
    let Some(settings) = settings else {
        return Ok(serde_json::to_string_pretty(&compose)?);
    };
    // 配了解析器时核心和 lego 都用它：入口的 ACME 目录、acme-dns 和 CAA 都靠这些名字。
    let resolvers = crate::entry_settings::resolver_ips(&settings.acme_resolvers)?;
    if !resolvers.is_empty() {
        // Docker 的 127.0.0.11 在前：容器名、插件别名只有它认识；它认不出的名字转发给配置的解析器。
        // 反过来先问真实解析器时，它们在区外名字上回 NXDOMAIN，glibc 不再回退，内部名就解析不到。
        let mut servers = vec!["127.0.0.11".to_owned()];
        servers.extend(resolvers);
        compose["services"]["core"]["dns"] = json!(servers);
    }
    compose["services"]["core"]["environment"]["WONTON_MATRIX_FEDERATION_ADDR"] =
        json!("entry.wonton.internal:8449");
    Ok(serde_json::to_string_pretty(&compose)?)
}

pub fn write(install_dir: &Path, settings: &EntrySettings) -> anyhow::Result<()> {
    let directory = install_dir.join("entry");
    std::fs::create_dir_all(&directory)?;
    let rendered = crate::entry_haproxy::render(&crate::entry_haproxy::Model::federation_only(
        &settings.domain,
        &settings.domain,
    ))
    .map_err(anyhow::Error::msg)?;
    crate::install::write_private(&directory.join("haproxy.cfg"), &rendered.config)?;
    std::fs::create_dir_all(directory.join("certs"))?;
    crate::install::write_private(
        &directory.join("certs/public.crt-list"),
        &rendered.public_crt_list,
    )?;
    crate::install::write_private(
        &directory.join("certs/lan.crt-list"),
        &rendered.lan_crt_list,
    )?;
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
        "certs/public.crt-list",
        "certs/lan.crt-list",
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
            acme_propagation_disable_ans: false,
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

    fn core_volumes(rendered: &str) -> Vec<String> {
        let volumes =
            &serde_json::from_str::<Value>(rendered).unwrap()["services"]["core"]["volumes"];
        // render 写的是字符串卷；断言只关心这两个挂载在不在，对象形式也按字符串比对。
        volumes
            .as_array()
            .unwrap()
            .iter()
            .map(|volume| match volume.as_str() {
                Some(text) => text.to_owned(),
                None => volume.to_string(),
            })
            .collect()
    }

    /// “不要名字”的安装也要能运行时签发：核心容器要挂 `entry/`（写 CSR、ACME 状态给 lego 按宿主
    /// 路径绑挂）和 `entry-input/`（读 CA）。两种模式挂载一致，签发路径才只有一条。
    #[test]
    fn core_mounts_entry_paths_in_every_mode() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("entry/certs")).unwrap();
        std::fs::write(dir.path().join("entry/certs/site.pem"), "pem").unwrap();
        for rendered in [
            render(crate::install::COMPOSE, None, dir.path()).unwrap(),
            render(crate::install::COMPOSE, Some(&settings()), dir.path()).unwrap(),
        ] {
            let volumes = core_volumes(&rendered);
            assert!(
                volumes.contains(
                    &"${WONTON_INSTALL_DIR}/entry:${WONTON_INSTALL_DIR}/entry".to_owned()
                ),
                "core 没挂 entry：{volumes:?}"
            );
            assert!(
                volumes.contains(
                    &"${WONTON_INSTALL_DIR}/entry-input:${WONTON_INSTALL_DIR}/entry-input:ro"
                        .to_owned()
                ),
                "core 没挂 entry-input：{volumes:?}"
            );
            assert_eq!(
                volumes
                    .iter()
                    .filter(|volume| volume.starts_with("${WONTON_INSTALL_DIR}/entry"))
                    .count(),
                2,
                "entry 挂载重复了：{volumes:?}"
            );
        }
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

    /// 联邦名单在 `secrets/tuwunel/wonton-fed.toml`，配对变化时核心原地重写（ADR-0050）：整个 `secrets/`
    /// 只读，这个子目录要单独挂成可写核心才写得了；旧的 `fed/` 挂载已废弃。
    #[test]
    fn core_can_rewrite_the_federation_list() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("entry/certs")).unwrap();
        std::fs::write(dir.path().join("entry/certs/site.pem"), "pem").unwrap();
        for rendered in [
            render(crate::install::COMPOSE, None, dir.path()).unwrap(),
            render(crate::install::COMPOSE, Some(&settings()), dir.path()).unwrap(),
        ] {
            // core 还挂着结构形式的 /dev 只读挂载，不是字符串；名单和密钥挂载都是字符串形式。
            let volumes: Vec<String> =
                serde_json::from_str::<Value>(&rendered).unwrap()["services"]["core"]["volumes"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter_map(|volume| volume.as_str().map(str::to_owned))
                    .collect();
            assert!(
                volumes.contains(
                    &"${WONTON_INSTALL_DIR}/secrets:${WONTON_INSTALL_DIR}/secrets:ro".to_owned()
                ),
                "secrets 根目录要只读：{volumes:?}"
            );
            assert!(
                volumes.contains(
                    &"${WONTON_INSTALL_DIR}/secrets/tuwunel:${WONTON_INSTALL_DIR}/secrets/tuwunel"
                        .to_owned()
                ),
                "联邦名单子目录要单独可写：{volumes:?}"
            );
            assert!(
                !volumes
                    .iter()
                    .any(|volume| volume.split(':').next().unwrap().ends_with("/fed")),
                "fed/ 挂载已废弃：{volumes:?}"
            );
        }
    }

    /// 入口和连接器由核心按设置建，compose 里只剩网络；edge 的子网和 isolated 照旧。
    #[test]
    fn networks_render_without_the_entry_services() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("entry/certs")).unwrap();
        std::fs::write(dir.path().join("entry/certs/site.pem"), "pem").unwrap();
        let compose: Value = serde_json::from_str(
            &render(crate::install::COMPOSE, Some(&settings()), dir.path()).unwrap(),
        )
        .unwrap();
        for service in ["entry", "connector"] {
            assert!(compose["services"].get(service).is_none(), "{service}");
        }
        assert_eq!(
            compose["networks"]["edge"]["ipam"]["config"][0]["subnet"],
            "10.90.0.0/24"
        );
        for network in ["clients", "edge"] {
            assert_eq!(compose["networks"][network]["internal"], true, "{network}");
            assert_eq!(
                compose["networks"][network]["driver_opts"]["com.docker.network.bridge.gateway_mode_ipv4"],
                "isolated",
                "{network}"
            );
            assert_eq!(
                compose["networks"][network]["driver_opts"]["com.docker.network.bridge.gateway_mode_ipv6"],
                "isolated",
                "{network}"
            );
        }
    }

    /// 没有入口设置也要接入 clients：8484 的绑定和主网关的排除地址在启动时就要定下来（ADR-0021）。
    #[test]
    fn compose_always_attaches_core_to_clients_network() {
        let dir = tempfile::tempdir().unwrap();
        let compose: Value =
            serde_json::from_str(&render(crate::install::COMPOSE, None, dir.path()).unwrap())
                .unwrap();
        assert!(compose["networks"]["clients"].is_object());
        assert_eq!(
            compose["services"]["core"]["networks"]["clients"],
            json!({"aliases": ["entry-gateway.wonton.internal"]})
        );
        assert_eq!(
            compose["services"]["core"]["environment"]["WONTON_ENTRY_LISTEN"],
            "entry-gateway.wonton.internal:8484"
        );
    }

    /// 主网关从一开始就不在入口地址上接连接：已攻破的入口读不到首次配对码。
    #[test]
    fn main_listener_excludes_clients_address_from_start() {
        let dir = tempfile::tempdir().unwrap();
        let compose: Value =
            serde_json::from_str(&render(crate::install::COMPOSE, None, dir.path()).unwrap())
                .unwrap();
        assert_eq!(
            compose["services"]["core"]["environment"]["WONTON_EXCLUDE_LISTEN"],
            "entry-gateway.wonton.internal"
        );
    }

    /// 入口配置由 `entry_haproxy` 生成：联邦名字进公网列表，配对前端照旧，crt-list 和配置一起落盘。
    #[test]
    fn write_publishes_the_haproxy_config_and_both_crt_lists() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("entry/certs")).unwrap();
        std::fs::write(dir.path().join("entry/certs/site.pem"), "pem").unwrap();
        write(dir.path(), &settings()).unwrap();
        let config = std::fs::read_to_string(dir.path().join("entry/haproxy.cfg")).unwrap();
        assert!(config.contains(
            "bind entry-edge.wonton.internal:8443 ssl crt-list /certs/public.crt-list strict-sni"
        ));
        assert!(config.contains("frontend paired"));
        assert_eq!(
            std::fs::read_to_string(dir.path().join("entry/certs/public.crt-list")).unwrap(),
            "/certs/site.pem home.example probe.home.example\n"
        );
        assert!(
            std::fs::read_to_string(dir.path().join("entry/certs/lan.crt-list"))
                .unwrap()
                .is_empty()
        );
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

    /// 隧道只绑核心在 core 网络上的地址（ADR-0021）：绑的别名只挂在那张网络上，被攻破的入口容器
    /// 在 clients 网络上发到隧道端口的包落不到套接字；core 网络上的别名能被 Docker 的内嵌 DNS
    /// 解析到核心在那里的地址。
    #[test]
    fn the_tunnel_binds_an_alias_that_only_exists_on_the_core_network() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("entry/certs")).unwrap();
        std::fs::write(dir.path().join("entry/certs/site.pem"), "pem").unwrap();
        let compose: Value = serde_json::from_str(
            &render(crate::install::COMPOSE, Some(&settings()), dir.path()).unwrap(),
        )
        .unwrap();
        let core = &compose["services"]["core"];
        assert_eq!(
            core["environment"]["WONTON_TUNNEL_BIND"],
            json!("tunnel.wonton.internal")
        );
        let aliases = core["networks"]["core"]["aliases"]
            .as_array()
            .expect("core 网络的 aliases");
        assert!(
            aliases.contains(&json!("tunnel.wonton.internal")),
            "core 网络上没有隧道别名：{aliases:?}"
        );
        assert_eq!(
            core["networks"]["clients"],
            json!({"aliases": ["entry-gateway.wonton.internal"]}),
            "入口别名要在 clients 网络上，不能和隧道别名同网络"
        );
    }
}
