//! 宿主网络的网页和 MCP 服务经 unix 套接字中继（ADR-0036）。

use std::path::Path;

use tokio::net::UnixListener;

use super::super::fake_docker::FakeDocker;
use super::super::languages::Languages;
use super::super::manifest::{Permission, PluginId};
use super::super::package::{PackageError, PluginPackage};
use super::manager::{manager_in_container, manager_with};
use super::package;

const HOST_WEBUI: &str =
    "  web:\n    image: nginx\n    network_mode: host\n    healthcheck: {test: ['CMD', 'true']}\n";

fn host_webui(port: u16) -> String {
    package(
        HOST_WEBUI,
        &format!(
            "  webui: {{service: web, port: {port}, sso: none}}\n  permissions: [network.host]"
        ),
    )
}

#[test]
fn reserved_relay_service_names_are_rejected() {
    for host in [true, false] {
        for name in ["relay-webui", "relay-mcp"] {
            let services = format!("{HOST_WEBUI}  {name}:\n    image: nginx\n");
            let yaml = package(
                &services,
                "  webui: {service: web, port: 8123, sso: none}\n  \
                 mcp: {service: web, port: 9000, transport: streamable-http}\n  \
                 permissions: [network.host]",
            );
            let yaml = if host {
                yaml
            } else {
                yaml.replace("    network_mode: host\n", "")
                    .replace("[network.host]", "[]")
            };
            let error = PluginPackage::parse(&yaml).unwrap_err();
            assert!(
                matches!(&error, PackageError::Service { service, .. } if service == name),
                "{name}, host={host}: {error}"
            );
        }
    }
}

#[test]
fn relay_service_name_prefixes_are_not_reserved() {
    let yaml = package(
        &format!(
            "{HOST_WEBUI}  relay-webui-custom:\n    image: nginx\n  \
             relay-mcp-custom:\n    image: nginx\n"
        ),
        "  webui: {service: web, port: 8123, sso: none}\n  \
         mcp: {service: web, port: 9000, transport: streamable-http}\n  \
         permissions: [network.host]",
    );
    let plan = super::super::render::dry_run(&PluginPackage::parse(&yaml).unwrap()).unwrap();
    let names: std::collections::BTreeSet<_> = plan.containers.iter().map(|c| &c.name).collect();
    assert_eq!(names.len(), plan.containers.len());
}

/// `/proc/net/tcp` 的一行：只关心本地地址、端口和状态。
fn proc_line(ip_hex: &str, port: u16, state: &str) -> String {
    format!(
        "   0: {ip_hex}:{port:04X} 00000000:0000 {state} 00000000:00000000 00:00000000 00000000     0        0 0 1 0000000000000000 100 0 0 10 0\n"
    )
}

fn proc_header() -> String {
    "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n".to_owned()
}

#[test]
fn proc_addresses_parse_ipv4_ipv6_and_wildcards() {
    let parse = super::super::host_relay::parse_proc_ip;
    assert_eq!(parse("0100007F").unwrap().to_string(), "127.0.0.1");
    assert_eq!(parse("00000000").unwrap().to_string(), "0.0.0.0");
    assert_eq!(
        parse("00000000000000000000000001000000")
            .unwrap()
            .to_string(),
        "::1"
    );
    assert_eq!(
        parse("00000000000000000000000000000000")
            .unwrap()
            .to_string(),
        "::"
    );
    assert_eq!(
        parse("0000000000000000FFFF00000100007F")
            .unwrap()
            .to_string(),
        "127.0.0.1"
    );
    assert!(parse("123").is_none());
}

#[tokio::test]
async fn a_loopback_only_host_webui_gets_no_firewall_notice() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager_in_container(engine, &dir);
    let socket = dir.path().join("apps/app-test/relay/webui.sock");
    serve(&socket);
    let port = free_port();
    docker.on_exec_output(move |container, argv, _| {
        assert_eq!(container, "wonton-app-test--relay-webui");
        assert_eq!(argv, ["cat", "/proc/net/tcp", "/proc/net/tcp6"]);
        (
            0,
            format!(
                "{}{}{}",
                proc_header(),
                proc_line("0100007F", port, "0A"),
                proc_line("00000000000000000000000001000000", port, "0A")
            ),
            String::new(),
        )
    });
    m.install(&host_webui(port), &[Permission::NetworkHost])
        .await
        .unwrap();
    let id = PluginId::parse("app.test").unwrap();
    let detail = m.installed_detail(&id, &Languages::default()).unwrap();
    assert!(detail.notices.is_empty(), "{:?}", detail.notices);
}

#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_real_loopback_listener_is_recognized_in_the_proc_table() {
    use crate::plugin::compose::PullPolicy;
    use crate::plugin::docker::Engine;
    use bollard::models::{ContainerCreateBody, HostConfig};

    let engine = Engine::connect().unwrap();
    engine.check_server().await.unwrap();
    const IMAGE: &str = "public.ecr.aws/docker/library/busybox:1.37";
    engine
        .ensure_image(IMAGE, PullPolicy::Missing, None)
        .await
        .unwrap();
    let port = free_port();
    let name = format!("wonton-priv-377-{}", std::process::id());
    engine.remove_container(&name).await.unwrap();
    engine
        .create_container(
            &name,
            ContainerCreateBody {
                image: Some(IMAGE.to_owned()),
                cmd: Some(vec![
                    "sh".to_owned(),
                    "-c".to_owned(),
                    format!("nc -l -p {port} -s 127.0.0.1"),
                ]),
                host_config: Some(HostConfig {
                    network_mode: Some("host".to_owned()),
                    ..Default::default()
                }),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    engine.start_container(&name).await.unwrap();
    let mut probed = None;
    for _ in 0..100 {
        probed = super::super::host_relay::listens_beyond_loopback(&engine, &name, port).await;
        if probed.is_some() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    engine.remove_container(&name).await.unwrap();
    assert_eq!(probed, Some(false));
}

#[tokio::test]
async fn a_wildcard_host_webui_still_gets_the_firewall_notice() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager_in_container(engine, &dir);
    let socket = dir.path().join("apps/app-test/relay/webui.sock");
    serve(&socket);
    let port = free_port();
    docker.on_exec_output(move |_, _, _| {
        (
            0,
            format!("{}{}", proc_header(), proc_line("00000000", port, "0A")),
            String::new(),
        )
    });
    m.install(&host_webui(port), &[Permission::NetworkHost])
        .await
        .unwrap();
    let id = PluginId::parse("app.test").unwrap();
    let detail = m.installed_detail(&id, &Languages::default()).unwrap();
    let notices: Vec<&str> = detail.notices.iter().map(|n| n.text.as_str()).collect();
    assert!(
        notices.contains(&"本机防火墙可能挡住了外来连接，局域网设备可能连不进来"),
        "{notices:?}"
    );
}

#[tokio::test]
async fn an_unknown_listener_does_not_claim_a_firewall() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager_in_container(engine, &dir);
    let socket = dir.path().join("apps/app-test/relay/webui.sock");
    serve(&socket);
    docker.on_exec_output(move |_, _, _| (0, proc_header(), String::new()));
    m.install(&host_webui(free_port()), &[Permission::NetworkHost])
        .await
        .unwrap();
    let id = PluginId::parse("app.test").unwrap();
    let detail = m.installed_detail(&id, &Languages::default()).unwrap();
    assert!(detail.notices.is_empty(), "{:?}", detail.notices);
}

/// 当前空着的回环端口：直连探测打的是宿主上的这个端口，测试结果不受本机其他服务影响。
fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// 在 `path` 上回 200，直到进程结束。
fn serve(path: &Path) {
    let _ = std::fs::create_dir_all(path.parent().unwrap());
    let listener = UnixListener::bind(path).unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let io = hyper_util::rt::TokioIo::new(stream);
                let service = hyper::service::service_fn(|_req| async {
                    Ok::<_, std::convert::Infallible>(hyper::Response::new(axum::body::Body::from(
                        "ok",
                    )))
                });
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(io, service)
                    .await;
            });
        }
    });
}

#[test]
fn host_webui_and_mcp_get_relays_on_the_host_network() {
    let yaml = package(
        "  web:\n    image: nginx\n    network_mode: host\n    healthcheck: {test: ['CMD', 'true']}\n  \
         mcp:\n    image: nginx\n    network_mode: host\n",
        "  webui: {service: web, port: 8123, sso: none}\n  mcp: {service: mcp, port: 9000, transport: streamable-http}\n  \
         permissions: [network.host]",
    );
    let plan = super::super::render::dry_run(&PluginPackage::parse(&yaml).unwrap()).unwrap();
    let relays = &plan.relays;
    assert_eq!(relays.len(), 2);
    assert_eq!(relays[0].service, "web");
    assert_eq!(relays[0].port, 8123);
    assert_eq!(relays[1].service, "mcp");
    for relay in relays {
        let name = super::super::host_relay::container(&Default::default(), "app-test", relay);
        let container = plan.containers.iter().find(|c| c.name == name).unwrap();
        let host = container.body.host_config.as_ref().unwrap();
        assert_eq!(host.network_mode.as_deref(), Some("host"));
        assert!(host.binds.as_ref().unwrap()[0].ends_with(":/s"));
        assert_eq!(
            container.body.cmd.as_ref().unwrap()[0],
            format!(
                "UNIX-LISTEN:/s/{}.sock,fork,mode=0660,unlink-early",
                relay.purpose.as_str()
            )
        );
    }
    let dir = "/wonton-dry-run/apps/app-test/relay";
    assert!(plan.dirs.contains_key(Path::new(dir)));
}

#[test]
fn services_in_the_plugin_network_get_no_relay() {
    let yaml = package(
        "  web:\n    image: nginx",
        "  webui: {service: web, port: 80, sso: none}\n  permissions: []",
    );
    let plan = super::super::render::dry_run(&PluginPackage::parse(&yaml).unwrap()).unwrap();
    assert!(plan.relays.is_empty());
}

#[tokio::test]
async fn installing_a_host_webui_goes_through_the_relay_and_reports_the_firewall() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager_with(engine, &dir);
    let socket = dir.path().join("apps/app-test/relay/webui.sock");
    serve(&socket);

    let port = free_port();
    docker.on_exec_output(move |_, _, _| {
        (
            0,
            format!("{}{}", proc_header(), proc_line("00000000", port, "0A")),
            String::new(),
        )
    });
    m.install(&host_webui(port), &[Permission::NetworkHost])
        .await
        .unwrap();
    let id = PluginId::parse("app.test").unwrap();
    let detail = m.installed_detail(&id, &Languages::default()).unwrap();
    let notices: Vec<&str> = detail.notices.iter().map(|n| n.text.as_str()).collect();
    assert!(
        notices.contains(&"本机防火墙可能挡住了外来连接，局域网设备可能连不进来"),
        "{notices:?}"
    );
}

#[tokio::test]
async fn a_host_mcp_service_is_reached_through_its_socket() {
    let yaml = package(
        "  mcp:\n    image: nginx\n    network_mode: host",
        "  mcp: {service: mcp, port: 9000, transport: streamable-http}\n  permissions: [network.host]",
    );
    let (_docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager_with(engine, &dir);
    m.install(&yaml, &[Permission::NetworkHost]).await.unwrap();

    let servers = m.mcp_servers();
    let [server] = &servers[..] else {
        panic!("{servers:?}")
    };
    assert_eq!(
        server.socket.as_deref(),
        Some(dir.path().join("apps/app-test/relay/mcp.sock").as_path())
    );
    assert_eq!(server.url, "http://localhost/mcp");
}

#[tokio::test]
async fn the_watch_brings_a_missing_relay_back() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager_with(engine, &dir);
    let socket = dir.path().join("apps/app-test/relay/webui.sock");
    serve(&socket);
    let port = free_port();
    m.install(&host_webui(port), &[Permission::NetworkHost])
        .await
        .unwrap();
    let relay = super::super::names::container_name("app-test", "relay-webui");
    assert!(docker.containers().contains(&relay));
    m.engine.remove_container(&relay).await.unwrap();
    assert!(!docker.containers().contains(&relay));
    m.watch_containers().await;
    assert!(docker.containers().contains(&relay));
}

#[tokio::test]
async fn uninstalling_takes_the_relay_with_it() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager_with(engine, &dir);
    let socket = dir.path().join("apps/app-test/relay/webui.sock");
    serve(&socket);
    let port = free_port();
    m.install(&host_webui(port), &[Permission::NetworkHost])
        .await
        .unwrap();
    let relay = super::super::names::container_name("app-test", "relay-webui");
    assert!(docker.containers().contains(&relay));
    m.uninstall(&PluginId::parse("app.test").unwrap(), false)
        .await
        .unwrap();
    assert!(!docker.containers().contains(&relay));
}

#[tokio::test]
async fn a_reachable_host_webui_gets_no_firewall_notice() {
    let (_docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager_with(engine, &dir);
    let socket = dir.path().join("apps/app-test/relay/webui.sock");
    serve(&socket);
    // 直连探测打的是 127.0.0.1 上的这个端口：真的有人监听，不该提示防火墙。
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        axum::serve(listener, axum::Router::new().fallback(|| async { "ok" }))
            .await
            .unwrap();
    });
    m.install(&host_webui(port), &[Permission::NetworkHost])
        .await
        .unwrap();
    let id = PluginId::parse("app.test").unwrap();
    let detail = m.installed_detail(&id, &Languages::default()).unwrap();
    assert!(detail.notices.is_empty(), "{:?}", detail.notices);
}

#[tokio::test]
async fn an_unreadable_network_does_not_claim_a_firewall() {
    let (_docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager_in_container(engine, &dir);
    let package = PluginPackage::parse(&host_webui(39871)).unwrap();
    assert_eq!(
        m.direct_webui_probe(&package.manifest, "wonton-missing")
            .await,
        None
    );
}
