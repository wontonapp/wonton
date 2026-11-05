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
    let (_docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager_with(engine, &dir);
    let socket = dir.path().join("apps/app-test/relay/webui.sock");
    serve(&socket);

    let port = free_port();
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
