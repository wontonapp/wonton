//! 通用能力的宿主网络中继：真实 unix HTTP 请求，以及丢失容器后的核对和重启恢复。

use std::path::Path;
use std::sync::Arc;

use tokio::net::UnixListener;

use super::super::fake_docker::FakeDocker;
use super::super::host_relay::{self, HostRelay, Purpose};
use super::super::manifest::Permission;
use super::super::package::{PackageError, PluginPackage};
use super::super::render::dry_run;
use super::super::upstreams::Endpoint;
use super::capability_deps_install::{consumer, id, provider};
use super::manager::manager_with;
use super::{langs, package};

const CAPABILITY: &str = "x-acme.search";

fn host_provider() -> String {
    provider("app.search", CAPABILITY, "")
        .replace(
            "    image: nginx",
            "    image: nginx\n    network_mode: host",
        )
        .replace("  permissions: []", "  permissions: [network.host]")
}

fn serve(socket: &Path) -> tokio::task::JoinHandle<()> {
    std::fs::create_dir_all(socket.parent().unwrap()).unwrap();
    let listener = UnixListener::bind(socket).unwrap();
    tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            tokio::spawn(async move {
                let service = hyper::service::service_fn(
                    |request: hyper::Request<hyper::body::Incoming>| async move {
                        assert_eq!(request.uri().path(), "/search");
                        Ok::<_, std::convert::Infallible>(hyper::Response::new(
                            axum::body::Body::from("host-capability"),
                        ))
                    },
                );
                hyper::server::conn::http1::Builder::new()
                    .serve_connection(hyper_util::rt::TokioIo::new(stream), service)
                    .await
                    .unwrap();
            });
        }
    })
}

async fn request(endpoint: &Endpoint, socket: &Path) {
    assert_eq!(endpoint, &Endpoint::unix(socket));
    let response = reqwest::Client::builder()
        .unix_socket(endpoint.socket().unwrap().to_path_buf())
        .no_proxy()
        .build()
        .unwrap()
        .get(endpoint.url("/search"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(response.text().await.unwrap(), "host-capability");
}

#[tokio::test]
async fn host_capability_routes_over_unix_and_recovers_after_watch_and_restore() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let manager = manager_with(engine.clone(), &dir);
    let relay = HostRelay::capability("api", 8080);
    let socket = host_relay::socket_path(
        &manager.cfg.relay_dir,
        &Default::default(),
        "app-search",
        &relay,
    );
    let server = serve(&socket);
    manager
        .install(&host_provider(), &[Permission::NetworkHost], langs())
        .await
        .unwrap();
    manager
        .install(&consumer("requires", CAPABILITY), &[], langs())
        .await
        .unwrap();
    let record = manager.store.load(&id("app.search")).unwrap().unwrap();
    assert_eq!(record.relays, vec![relay.clone()]);
    let name = host_relay::container(&Default::default(), "app-search", &relay);
    let created = docker.created(&name).unwrap();
    assert_eq!(created["Image"], host_relay::image());
    assert_eq!(created["HostConfig"]["NetworkMode"], "host");
    assert_eq!(created["Cmd"][1], "TCP:127.0.0.1:8080");
    let route = manager
        .capability_route(&id("app.test"), CAPABILITY, &langs())
        .await
        .unwrap();
    assert_eq!(route.provider, id("app.search"));
    request(&route.endpoint, &socket).await;

    manager.engine.remove_container(&name).await.unwrap();
    assert!(!docker.containers().contains(&name));
    manager.watch_containers().await;
    assert!(docker.containers().contains(&name));
    assert_eq!(docker.creations(&name), 2);
    let route = manager
        .capability_route(&id("app.test"), CAPABILITY, &langs())
        .await
        .unwrap();
    request(&route.endpoint, &socket).await;

    manager.engine.remove_container(&name).await.unwrap();
    drop(manager);
    let restarted = manager_with(engine, &dir);
    restarted.restore().await.unwrap();
    assert!(docker.containers().contains(&name));
    assert_eq!(docker.creations(&name), 3);
    let route = restarted
        .capability_route(&id("app.test"), CAPABILITY, &langs())
        .await
        .unwrap();
    request(&route.endpoint, &socket).await;
    server.abort();
}

#[test]
fn capability_relays_are_unique_per_host_service_and_port() {
    let yaml = package(
        "  api:\n    image: nginx\n    network_mode: host\n  other:\n    image: nginx\n    network_mode: host\n  bridge:\n    image: nginx\n",
        "  permissions: [network.host]\n  provides:\n    x-acme.search: {service: api, port: 8080}\n    x-acme.index: {service: api, port: 8080}\n    x-acme.metrics: {service: api, port: 8081}\n    x-acme.other: {service: other, port: 8080}\n    x-acme.bridge: {service: bridge, port: 8080}\n",
    )
    .replace("spec: '0.5'", "spec: '0.11'");
    let plan = dry_run(&PluginPackage::parse(&yaml, &langs()).unwrap(), &langs()).unwrap();
    assert_eq!(plan.relays.len(), 3);
    for (service, port) in [("api", 8080), ("api", 8081), ("other", 8080)] {
        let relay = HostRelay::capability(service, port);
        assert!(plan.relays.contains(&relay));
        assert_eq!(relay.purpose, Purpose::Capability);
        let name = host_relay::container(&Default::default(), "app-test", &relay);
        let container = plan.containers.iter().find(|c| c.name == name).unwrap();
        let socket = host_relay::socket_path(
            Path::new(crate::config::DEFAULT_RELAY_DIR),
            &Default::default(),
            "app-test",
            &relay,
        );
        assert_eq!(
            container.body.cmd.as_ref().unwrap()[0],
            format!(
                "UNIX-LISTEN:{},fork,mode=0660,unlink-early",
                socket.display()
            )
        );
    }
}

#[test]
fn only_generated_capability_relay_names_are_reserved() {
    let generated = HostRelay::capability("api", 8080).service_name();
    let collision = host_provider().replace(
        "services:\n",
        &format!("services:\n  {generated}:\n    image: nginx\n"),
    );
    assert!(matches!(
        PluginPackage::parse(&collision, &langs()),
        Err(PackageError::Service { service, .. }) if service == generated
    ));
    let unrelated = collision.replace(&format!("  {generated}:"), "  relay-capability-unrelated:");
    PluginPackage::parse(&unrelated, &langs()).unwrap();
    let different_port =
        collision.replace(&format!("  {generated}:"), "  relay-capability-api-8081:");
    PluginPackage::parse(&different_port, &langs()).unwrap();
}

#[tokio::test]
async fn a_bridge_capability_keeps_its_ordinary_service_address() {
    let (_docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let manager = manager_with(engine, &dir);
    let source = provider("app.search", CAPABILITY, "");
    manager.install(&source, &[], langs()).await.unwrap();
    manager
        .install(&consumer("requires", CAPABILITY), &[], langs())
        .await
        .unwrap();
    let route = manager
        .capability_route(&id("app.test"), CAPABILITY, &langs())
        .await
        .unwrap();
    let slug = id("app.search").slug();
    let address = manager
        .service_address(
            &manager.cfg.instance.container(&slug, "api"),
            &super::super::names::service_host(&slug, "api"),
            8080,
            &manager.cfg.instance.network(&slug),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(route.endpoint, Endpoint::Tcp(address));
    assert!(
        manager
            .store
            .load(&id("app.search"))
            .unwrap()
            .unwrap()
            .relays
            .is_empty()
    );
}

/// 宿主网络的服务默认没有 `NET_RAW`：起不来时错误里点名缺的能力和补法，作者不用猜。
#[tokio::test]
async fn a_host_network_failure_names_the_dropped_capability() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = Arc::new(manager_with(engine, &dir));
    let compose = package(
        "  proxy:\n    image: nginx\n    network_mode: host\n    \
         command: ['sh', '-c', 'ping -c1 8.8.8.8']",
        "  permissions: [network.host]",
    );
    docker.set_state(
        &super::super::names::container_name("app-test", "proxy"),
        serde_json::json!({"Status": "exited", "ExitCode": 1}),
    );
    let error = m
        .install(&compose, &[Permission::NetworkHost], langs())
        .await
        .unwrap_err();
    let message = error.to_string();
    assert!(message.contains("NET_RAW"), "{message}");
    assert!(message.contains("cap_add: [NET_RAW]"), "{message}");
    assert!(message.contains("cap.NET_RAW"), "{message}");
}
