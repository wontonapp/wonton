//! 插件端口解析契约：安装记录决定声明，Docker inspect 决定插件网络里的地址。

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::extract::State;
use axum::http::{StatusCode, Uri};
use axum::{Json, Router};
use serde_json::{Value, json};
use wonton_protocol::PortProtocol;
use wonton_tunnel::{PortResolveError, PortResolver};

use super::super::docker::Engine;
use super::super::store::InstalledRecord;
use super::super::{PluginId, PluginManager, PluginPackage, PluginStatus};
use super::manager::{manager_in_container, manager_with};
use crate::tunnel::PluginPortResolver;

#[derive(Default)]
struct Inspections {
    replies: BTreeMap<String, (StatusCode, Value)>,
    containers: Vec<String>,
}

/// 只回答 inspect，其他 Docker 请求会让测试失败；返回真实 Engine 使用的 Docker JSON。
#[derive(Clone, Default)]
struct FakeEngine(Arc<Mutex<Inspections>>);

impl FakeEngine {
    async fn start() -> (Self, Engine) {
        let fake = Self::default();
        let app = Router::new().fallback(inspect).with_state(fake.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (fake, Engine::http_for_test(&format!("http://{address}")))
    }

    fn reply(&self, container: &str, status: StatusCode, body: Value) {
        self.0
            .lock()
            .unwrap()
            .replies
            .insert(container.to_owned(), (status, body));
    }

    fn inspected(&self) -> Vec<String> {
        self.0.lock().unwrap().containers.clone()
    }
}

async fn inspect(State(fake): State<FakeEngine>, uri: Uri) -> (StatusCode, Json<Value>) {
    let container = uri
        .path()
        .strip_suffix("/json")
        .and_then(|path| path.rsplit_once("/containers/"))
        .map(|(_, container)| container)
        .expect("解析器只能查询容器地址");
    let mut inspections = fake.0.lock().unwrap();
    inspections.containers.push(container.to_owned());
    let (status, body) = inspections.replies.get(container).cloned().unwrap_or((
        StatusCode::NOT_FOUND,
        json!({"message": "missing container"}),
    ));
    (status, Json(body))
}

fn source(id: &str, ports: Option<&str>) -> String {
    let tunnel = ports.map_or_else(String::new, |ports| {
        format!("  tunnel:\n    ports: [{ports}]\n")
    });
    format!(
        "services:\n  frontend: {{image: example/frontend}}\n  media: {{image: example/media}}\n\
         x-wonton:\n  spec: '0.11'\n  id: {id}\n  version: 1.0.0\n  kind: app\n  \
         title: {{en: Test}}\n  description: {{en: Test}}\n  permissions: []\n{tunnel}"
    )
}

struct Fixture {
    dir: tempfile::TempDir,
    engine: FakeEngine,
    manager: Arc<PluginManager>,
    resolver: PluginPortResolver,
}

impl Fixture {
    async fn new(in_container: bool) -> Self {
        let (engine, client) = FakeEngine::start().await;
        let dir = tempfile::tempdir().unwrap();
        let manager = if in_container {
            manager_in_container(client, &dir)
        } else {
            manager_with(client, &dir)
        };
        let manager = Arc::new(manager);
        let resolver = PluginPortResolver::new(manager.clone());
        Self {
            dir,
            engine,
            manager,
            resolver,
        }
    }

    fn installed(&self, id: &str, ports: Option<&str>, status: PluginStatus) {
        let source = source(id, ports);
        let package = PluginPackage::parse(&source, &super::langs()).unwrap();
        self.manager
            .store
            .save(&InstalledRecord {
                id: package.manifest.id.clone(),
                version: "1.0.0".into(),
                source,
                files: Default::default(),
                granted_permissions: Vec::new(),
                enabled_permissions: Vec::new(),
                limits: Default::default(),
                lan_webui: false,
                relays: Vec::new(),
                firewalled: false,
                proxied: false,
                oidc_client_secret: None,
                installed_at: 1,
                entry_label: None,
                clients_entries: Default::default(),
                setup_done_ever: false,
                origin: None,
                written_by: None,
                ask_every_update: false,
                family: false,
                settings: Default::default(),
                settings_changed_ms: None,
                static_files: Default::default(),
                hosted: Vec::new(),
                gpu_vendor: None,
            })
            .unwrap();
        self.manager
            .register_for_test(package.manifest, status, None);
    }

    fn address(&self, container: &str, network: &str, endpoint: Value) {
        self.engine.reply(
            container,
            StatusCode::OK,
            json!({"NetworkSettings": {"Networks": {network: endpoint}}}),
        );
    }

    async fn resolve(
        &self,
        id: &str,
        protocol: PortProtocol,
    ) -> Result<SocketAddr, PortResolveError> {
        self.resolver.resolve(id, 1024, protocol).await
    }
}

#[tokio::test]
async fn declarations_select_the_exact_plugin_service_and_network() {
    let fixture = Fixture::new(true).await;
    fixture.installed(
        "app.mine",
        Some("{service: frontend, port: 1024}, {service: media, port: 1024, protocol: udp}"),
        PluginStatus::Running,
    );
    fixture.installed(
        "app.other",
        Some("{service: frontend, port: 1024}, {service: media, port: 4096}"),
        PluginStatus::Running,
    );
    fixture.engine.reply(
        "wonton-app-mine--frontend",
        StatusCode::OK,
        json!({"NetworkSettings": {"Networks": {
            "wonton-app-other": {"IPAddress": "192.0.2.99"},
            "wonton-app-mine": {"IPAddress": "192.0.2.10"}
        }}}),
    );
    fixture.address(
        "wonton-app-mine--media",
        "wonton-app-mine",
        json!({"IPAddress": "192.0.2.11"}),
    );
    fixture.address(
        "wonton-app-other--frontend",
        "wonton-app-other",
        json!({"IPAddress": "192.0.2.20"}),
    );
    for (plugin, protocol, expected) in [
        ("app.mine", PortProtocol::Tcp, "192.0.2.10:1024"),
        ("app.mine", PortProtocol::Udp, "192.0.2.11:1024"),
        ("app.other", PortProtocol::Tcp, "192.0.2.20:1024"),
    ] {
        assert_eq!(
            fixture.resolve(plugin, protocol).await.unwrap(),
            expected.parse().unwrap()
        );
    }
    assert_eq!(
        fixture
            .resolver
            .resolve("app.mine", 4096, PortProtocol::Tcp)
            .await,
        Err(PortResolveError::PortNotDeclared)
    );
    assert_eq!(
        fixture.engine.inspected(),
        [
            "wonton-app-mine--frontend",
            "wonton-app-mine--media",
            "wonton-app-other--frontend"
        ]
    );
}

#[tokio::test]
async fn absent_records_and_request_hostnames_are_not_installed() {
    let fixture = Fixture::new(false).await;
    // 即使内存表残留运行状态，也必须有真正的安装记录。
    let package = PluginPackage::parse(&source("app.stale", None), &super::langs()).unwrap();
    fixture
        .manager
        .register_for_test(package.manifest, PluginStatus::Running, None);
    for plugin in [
        "app.missing",
        "app.stale",
        "localhost",
        "127.0.0.1",
        "[::1]",
        "169.254.169.254",
        "hydra.wonton.internal",
        "../app.mine",
        "",
    ] {
        assert_eq!(
            fixture.resolve(plugin, PortProtocol::Tcp).await,
            Err(PortResolveError::PluginNotInstalled),
            "{plugin}"
        );
    }
    assert!(fixture.engine.inspected().is_empty());
}

#[tokio::test]
async fn declarations_and_protocols_are_checked_before_running_state() {
    let fixture = Fixture::new(false).await;
    fixture.installed("app.none", None, PluginStatus::Running);
    fixture.installed(
        "app.stopped",
        Some("{service: frontend, port: 1024}, {service: media, port: 2048, protocol: future}"),
        PluginStatus::Stopped,
    );
    for (plugin, port, protocol) in [
        ("app.none", 1024, PortProtocol::Tcp),
        ("app.stopped", 1025, PortProtocol::Tcp),
        ("app.stopped", 1024, PortProtocol::Udp),
        ("app.stopped", 2048, PortProtocol::Other),
    ] {
        assert_eq!(
            fixture.resolver.resolve(plugin, port, protocol).await,
            Err(PortResolveError::PortNotDeclared)
        );
    }
    assert_eq!(
        fixture.resolve("app.stopped", PortProtocol::Tcp).await,
        Err(PortResolveError::PluginNotRunning)
    );
    assert!(fixture.engine.inspected().is_empty());
}

#[tokio::test]
async fn installed_but_unregistered_plugins_are_not_running() {
    let fixture = Fixture::new(false).await;
    fixture.installed(
        "app.mine",
        Some("{service: frontend, port: 1024}"),
        PluginStatus::Running,
    );
    let id = PluginId::parse("app.mine").unwrap();
    fixture.manager.plugins.write().unwrap().remove(&id);
    assert_eq!(
        fixture.resolve("app.mine", PortProtocol::Tcp).await,
        Err(PortResolveError::PluginNotRunning)
    );
    assert!(fixture.engine.inspected().is_empty());
}

#[tokio::test]
async fn corrupt_and_unreadable_records_are_upstream_failures() {
    let fixture = Fixture::new(false).await;
    fixture.installed(
        "app.mine",
        Some("{service: frontend, port: 1024}"),
        PluginStatus::Stopped,
    );
    let path = fixture.dir.path().join("plugins/app.mine.json");
    std::fs::write(&path, "{ broken").unwrap();
    assert_eq!(
        fixture.resolve("app.mine", PortProtocol::Tcp).await,
        Err(PortResolveError::UpstreamUnavailable)
    );
    std::fs::remove_file(&path).unwrap();
    // 目录在 root 下也读不成记录，不依赖 chmod 的权限行为。
    std::fs::create_dir(&path).unwrap();
    assert_eq!(
        fixture.resolve("app.mine", PortProtocol::Tcp).await,
        Err(PortResolveError::UpstreamUnavailable)
    );
    assert!(fixture.engine.inspected().is_empty());
}

#[tokio::test]
async fn malformed_packages_and_mismatched_identities_are_upstream_failures() {
    let fixture = Fixture::new(false).await;
    fixture.installed("app.mine", None, PluginStatus::Running);
    let id = PluginId::parse("app.mine").unwrap();
    let mut record = fixture.manager.store.load(&id).unwrap().unwrap();
    for source in [
        "services: [broken".to_owned(),
        source("app.other", Some("{service: frontend, port: 1024}")),
    ] {
        record.source = source;
        fixture.manager.store.save(&record).unwrap();
        assert_eq!(
            fixture.resolve("app.mine", PortProtocol::Tcp).await,
            Err(PortResolveError::UpstreamUnavailable)
        );
    }
    record.id = PluginId::parse("app.other").unwrap();
    let path = fixture.dir.path().join("plugins/app.mine.json");
    std::fs::write(path, serde_json::to_vec(&record).unwrap()).unwrap();
    assert_eq!(
        fixture.resolve("app.mine", PortProtocol::Tcp).await,
        Err(PortResolveError::UpstreamUnavailable)
    );
    assert!(fixture.engine.inspected().is_empty());
}

#[tokio::test]
async fn missing_or_invalid_plugin_network_addresses_are_upstream_failures() {
    for in_container in [false, true] {
        let fixture = Fixture::new(in_container).await;
        fixture.installed(
            "app.mine",
            Some("{service: frontend, port: 1024}"),
            PluginStatus::Running,
        );
        for body in [
            json!({}),
            json!({"NetworkSettings": {}}),
            json!({"NetworkSettings": {"Networks": {}}}),
            json!({"NetworkSettings": {"Networks": {
                "wonton-app-other": {"IPAddress": "192.0.2.20"}
            }}}),
            json!({"NetworkSettings": {"Networks": {"wonton-app-mine": {}}}}),
            json!({"NetworkSettings": {"Networks": {"wonton-app-mine": {
                "IPAddress": "", "GlobalIPv6Address": ""
            }}}}),
            json!({"NetworkSettings": {"Networks": {"wonton-app-mine": {
                "IPAddress": "localhost"
            }}}}),
            json!({"NetworkSettings": {"Networks": {"wonton-app-mine": {
                "IPAddress": "192.0.2.10:9999"
            }}}}),
        ] {
            fixture
                .engine
                .reply("wonton-app-mine--frontend", StatusCode::OK, body.clone());
            assert_eq!(
                fixture.resolve("app.mine", PortProtocol::Tcp).await,
                Err(PortResolveError::UpstreamUnavailable),
                "in_container={in_container}: {body}"
            );
        }
        for status in [StatusCode::NOT_FOUND, StatusCode::INTERNAL_SERVER_ERROR] {
            fixture.engine.reply(
                "wonton-app-mine--frontend",
                status,
                json!({"message": "inspect failed"}),
            );
            assert_eq!(
                fixture.resolve("app.mine", PortProtocol::Tcp).await,
                Err(PortResolveError::UpstreamUnavailable)
            );
        }
    }
}

#[tokio::test]
async fn inspected_ipv4_and_ipv6_addresses_keep_the_declared_port() {
    let fixture = Fixture::new(true).await;
    fixture.installed(
        "app.mine",
        Some("{service: frontend, port: 1024}"),
        PluginStatus::Running,
    );
    for (endpoint, expected) in [
        (json!({"IPAddress": "192.0.2.10"}), "192.0.2.10:1024"),
        (json!({"IPAddress": "2001:db8::10"}), "[2001:db8::10]:1024"),
        (
            json!({"IPAddress": "", "GlobalIPv6Address": "2001:db8::20"}),
            "[2001:db8::20]:1024",
        ),
        (
            json!({"GlobalIPv6Address": "2001:db8::30"}),
            "[2001:db8::30]:1024",
        ),
    ] {
        fixture.address("wonton-app-mine--frontend", "wonton-app-mine", endpoint);
        assert_eq!(
            fixture
                .resolve("app.mine", PortProtocol::Tcp)
                .await
                .unwrap(),
            expected.parse().unwrap()
        );
    }
}
