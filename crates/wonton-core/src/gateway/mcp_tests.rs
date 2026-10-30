//! 经网关访问 MCP 总线：按来源地址授权，Agent 用真实的 MCP 客户端（rmcp，Streamable HTTP）列工具、调工具；
//! 主人回答确认只能经隧道。总线自己的规则见 `mcp::tests`。

use std::fs;
use std::net::SocketAddr;
use std::os::unix::fs::MetadataExt;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use http_body_util::BodyExt;
use hyper::header::{self, HeaderValue};
use hyper::{Method, StatusCode};
use iroh::SecretKey;
use rmcp::model::{CallToolRequestParams, ClientCapabilities, ClientConfig, Implementation};
use rmcp::service::{RoleClient, RunningService};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::{ClientLifecycleMode, ClientServiceExt};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tower::ServiceExt;

use super::tests::{CORE, TOKEN, owner};
use super::tunnel_tests::loopback_tunnel;
use super::{ERROR_HEADER, Gateway, TunnelPeer, router};
use crate::config::{PLUGIN_MCP_HOST, PublicUrls};
use crate::http::HttpClient;
use crate::mcp::test_support::{Cards, Notes, serve_notes};
use crate::mcp::{Bus, Confirmations};
use crate::plugin::docker::Engine;
use crate::plugin::image_worker::ImageWorker;
use crate::plugin::lifecycle::LifecycleConfig;
use crate::plugin::store::Store;
use crate::plugin::{Permission, PluginManager, PluginPackage, PluginStatus};

const NOTES: &str = "name: notes
services:
  mcp:
    image: example/notes
x-wonton:
  id: app.example.notes
  version: 0.1.0
  kind: app
  title: {zh: 笔记, en: Notes}
  description: {zh: 记笔记, en: Notes}
  mcp: {service: mcp, port: 8080, transport: streamable-http}
  permissions: []
";

struct Home {
    addr: SocketAddr,
    gateway: Arc<Gateway>,
    cards: Arc<Cards>,
    _dir: tempfile::TempDir,
}

/// 核心加上总线；`permissions` 是本机（127.0.0.0/8）冒充的插件网络得到的权限，`None` 时本机不是插件网络。
async fn start(permissions: Option<Vec<Permission>>) -> Home {
    let dir = tempfile::tempdir().unwrap();
    let owner_ids = fs::metadata(dir.path()).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let public = PublicUrls {
        scheme: "http".into(),
        base_domain: CORE.into(),
        port: addr.port(),
    };
    let lifecycle = LifecycleConfig {
        public: public.clone(),
        apps_dir: dir.path().join("apps"),
        shares_dir: dir.path().join("shares"),
        core_container: None,
        tz: "UTC".into(),
        puid: owner_ids.uid(),
        pgid: owner_ids.gid(),
        install_timeout: Duration::from_secs(5),
        owner: "owner".into(),
        gateway_port: addr.port(),
        instance: Default::default(),
    };
    let store = Store::open(dir.path().join("plugins")).unwrap();
    let http = HttpClient::new();
    let manager = Arc::new(PluginManager::new(
        Engine::unreachable_for_test(),
        store,
        http.clone(),
        None,
        lifecycle,
        ImageWorker::in_process(),
    ));
    let manifest = PluginPackage::parse(NOTES).unwrap().manifest;
    let id = manifest.id.clone();
    manager.register_for_test(manifest, PluginStatus::Running, None);
    manager.mcp_for_test(&id, serve_notes(Notes::default()).await);
    if let Some(permissions) = permissions {
        manager.register_source_for_test("com.example.agent", "127.0.0.0/8", permissions);
    }
    let cards = Arc::new(Cards::default());
    let confirmations = Arc::new(Confirmations::new(cards.clone(), Duration::from_secs(60)));
    let audit = dir.path().join("audit.jsonl");
    let bus = Bus::new(manager.clone(), None, confirmations, audit);
    let gateway = Gateway::new(
        public,
        owner(),
        TOKEN.into(),
        manager,
        None,
        http,
        loopback_tunnel(dir.path()).await,
    )
    .with_mcp(Some(bus.endpoint()));
    let gateway = Arc::new(gateway);
    let app = router(gateway.clone());
    tokio::spawn(async move {
        let service = app.into_make_service_with_connect_info::<SocketAddr>();
        axum::serve(listener, service).await.unwrap();
    });
    Home {
        addr,
        gateway,
        cards,
        _dir: dir,
    }
}

impl Home {
    fn mcp_url(&self) -> String {
        format!("http://{PLUGIN_MCP_HOST}:{}/mcp", self.addr.port())
    }

    /// 像 Agent 一样连上总线：`mcp.wonton.internal` 解析到网关。
    async fn agent(&self) -> RunningService<RoleClient, ClientConfig> {
        let http = reqwest::Client::builder()
            .no_proxy()
            .resolve(PLUGIN_MCP_HOST, self.addr)
            .build()
            .unwrap();
        let config = StreamableHttpClientTransportConfig::with_uri(self.mcp_url());
        let transport = StreamableHttpClientTransport::with_client(http, config);
        let info = ClientConfig::new(
            ClientCapabilities::default(),
            Implementation::new("test-agent", "0"),
        );
        let lifecycle = ClientLifecycleMode::Auto {
            preferred_versions: vec![rmcp::model::ProtocolVersion::V_2026_07_28],
            legacy_version: Some(rmcp::model::ProtocolVersion::V_2025_11_25),
        };
        info.serve_with_lifecycle(transport, lifecycle)
            .await
            .unwrap()
    }

    /// 在网关进程内发一个请求；`peer` 是隧道认证过的设备。
    async fn send(
        &self,
        path: &str,
        body: &str,
        peer: Option<TunnelPeer>,
        headers: &[(&str, &str)],
    ) -> (StatusCode, Option<String>, Value) {
        let mut request = hyper::Request::builder()
            .method(Method::POST)
            .uri(path)
            .header(header::HOST, format!("{CORE}:{}", self.addr.port()))
            .header(header::CONTENT_TYPE, "application/json");
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        let mut request = request.body(Body::from(body.to_owned())).unwrap();
        if let Some(peer) = peer {
            request.extensions_mut().insert(peer);
        }
        let response = router(self.gateway.clone()).oneshot(request).await.unwrap();
        let status = response.status();
        let kind = response
            .headers()
            .get(ERROR_HEADER)
            .map(|v| v.to_str().unwrap().to_owned());
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            kind,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    fn paired_device(&self) -> TunnelPeer {
        let id = SecretKey::generate().public();
        self.gateway.tunnel.devices.add(id, None).unwrap();
        TunnelPeer(id)
    }
}

fn text(result: &rmcp::model::CallToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(|c| c.as_text().map(|t| t.text.clone()))
        .collect()
}

#[tokio::test]
async fn agents_with_the_tools_permission_use_the_bus_through_the_gateway() {
    let home = start(Some(vec![Permission::Tools])).await;
    let agent = home.agent().await;
    let tools = agent.peer().list_all_tools().await.unwrap();
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
    assert!(names.contains(&"wonton__list_plugins"), "{names:?}");
    assert!(names.contains(&"app-example-notes__search"), "{names:?}");
    let request = CallToolRequestParams::new("app-example-notes__search")
        .with_arguments(json!({"name": "x"}).as_object().cloned().unwrap());
    let result = agent.peer().call_tool(request).await.unwrap();
    assert_eq!(text(&result), "找到 x.txt");
}

#[tokio::test]
async fn only_plugin_networks_with_the_tools_permission_get_in() {
    async fn probe(home: &Home) -> hyper::Response<hyper::body::Incoming> {
        let url = format!("http://{}/mcp", home.addr);
        let request = hyper::Request::builder()
            .method(Method::POST)
            .uri(url)
            .header(
                header::HOST,
                format!("{PLUGIN_MCP_HOST}:{}", home.addr.port()),
            )
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from("{}"))
            .unwrap();
        HttpClient::new().send(request).await.unwrap()
    }
    let without = start(Some(vec![Permission::Models])).await;
    let response = probe(&without).await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        response.headers()[ERROR_HEADER],
        HeaderValue::from_static("permission_denied")
    );
    let outsider = start(None).await;
    let response = probe(&outsider).await;
    assert_eq!(
        response.headers()[ERROR_HEADER],
        HeaderValue::from_static("unauthenticated")
    );
}

#[tokio::test]
async fn only_paired_devices_answer_confirmations() {
    let home = start(Some(vec![Permission::Tools])).await;
    let agent = home.agent().await;
    let peer = agent.peer().clone();
    let call = tokio::spawn(async move {
        let request = CallToolRequestParams::new("app-example-notes__delete")
            .with_arguments(json!({"name": "y"}).as_object().cloned().unwrap());
        peer.call_tool(request).await.unwrap()
    });
    let card = home.cards.wait_for(1).await;
    let path = wonton_protocol::answer_path(&card.id);
    let approve = r#"{"answer": "approve"}"#;
    let device = home.paired_device();

    // 局域网请求（没有隧道身份），以及插件网页借 WebView 发的跨站请求，都答不了。
    let (status, kind, _) = home.send(&path, approve, None, &[]).await;
    assert_eq!(
        (status, kind.as_deref()),
        (StatusCode::UNAUTHORIZED, Some("unauthenticated"))
    );
    let plugin_page = [("origin", "http://app-example-notes.wonton.localhost")];
    let (status, kind, _) = home.send(&path, approve, Some(device), &plugin_page).await;
    assert_eq!(
        (status, kind.as_deref()),
        (StatusCode::FORBIDDEN, Some("cross_site"))
    );
    let (status, kind, _) = home.send(&path, "yes", Some(device), &[]).await;
    assert_eq!(
        (status, kind.as_deref()),
        (StatusCode::BAD_REQUEST, Some("invalid_input"))
    );
    assert!(home.cards.states(&card.id).is_empty(), "还没有人回答");

    let (status, _, reply) = home.send(&path, approve, Some(device), &[]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(reply["state"]["type"], "running");
    assert_eq!(text(&call.await.unwrap()), "删掉了 y.txt");
    let (status, kind, _) = home.send(&path, approve, Some(device), &[]).await;
    assert_eq!(
        (status, kind.as_deref()),
        (StatusCode::CONFLICT, Some("already_answered"))
    );
    let missing = wonton_protocol::answer_path("nope");
    let (status, kind, _) = home.send(&missing, approve, Some(device), &[]).await;
    assert_eq!(
        (status, kind.as_deref()),
        (StatusCode::NOT_FOUND, Some("not_found"))
    );
}
