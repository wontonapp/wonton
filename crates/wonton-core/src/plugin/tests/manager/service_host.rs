//! 核心在容器里时网页和 MCP 上游用的稳定服务域名。

use serde_json::json;

use super::super::super::PluginId;
use super::super::super::fake_docker::FakeDocker;
use super::super::super::names::container_name;
use super::super::super::store::{InstalledRecord, Store};
use super::super::super::upstreams::Endpoint;
use super::super::package;
use super::manager_in_container;

/// 核心在容器里时，网页和 MCP 上游连服务的稳定域名 `服务名.<slug>.wonton.internal`（上游看到的 Host 就是它，
/// 作者经 `WONTON_SERVICE_DOMAIN` 把它写进白名单）。
#[tokio::test]
async fn in_a_container_the_webui_and_mcp_upstreams_use_the_service_host() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let compose = package(
        "  web:\n    image: nginx",
        "  permissions: []\n  webui: {service: web, port: 8123, sso: none}\n  \
         mcp: {service: web, port: 9000, transport: streamable-http}",
    );
    Store::open(dir.path().join("plugins"))
        .unwrap()
        .save(&InstalledRecord {
            lifecycle: Default::default(),
            lan_webui: false,
            relays: Vec::new(),
            firewalled: false,
            proxied: false,
            bundled: false,
            id: PluginId::parse("app.test").unwrap(),
            version: "1.0.0".into(),
            source: compose,
            files: Default::default(),
            limits: Default::default(),
            granted_permissions: Vec::new(),
            enabled_permissions: Vec::new(),
            devices: Vec::new(),
            oidc_client_secret: None,
            installed_at: 1,
            entry_label: None,
            clients_entries: Default::default(),
            setup_done_ever: false,
            origin: None,
            written_by: None,
            ask_every_update: false,
            settings: Default::default(),
            settings_changed_ms: None,
            static_files: Default::default(),
            hosted: Vec::new(),
            gpu_vendor: None,
        })
        .unwrap();
    docker.add_container(&container_name("app-test", "web"), "app.test");
    docker.add_container("wonton-core", "app.test");
    docker.add_network(
        "wonton-app-test",
        "app.test",
        json!({"Internal": true, "IPAM": {"Config": [{
            "Subnet": "172.30.0.0/24", "IPRange": "172.30.0.0/25", "Gateway": "172.30.0.1",
        }]}, "Options": {
            "com.docker.network.bridge.gateway_mode_ipv4": "isolated",
            "com.docker.network.bridge.gateway_mode_ipv6": "isolated",
        }}),
    );
    let m = manager_in_container(engine, &dir);
    m.restore().await.unwrap();
    assert_eq!(
        m.route("app-test", "/").unwrap().upstream,
        Some(Endpoint::Tcp(
            "web.app-test.wonton.internal:8123".to_owned()
        ))
    );
    assert_eq!(
        m.mcp_servers()[0].url,
        "http://web.app-test.wonton.internal:9000/mcp"
    );
}
