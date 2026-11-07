//! 经网关访问 MCP 总线：按来源地址授权，Agent 用真实的 MCP 客户端（rmcp，Streamable HTTP）列工具、调工具；
//! 主人回答确认只能用已配对设备或主人令牌。总线自己的规则见 `mcp::tests`。

use std::fs;
use std::net::SocketAddr;
use std::os::unix::fs::MetadataExt;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use crate::plugin::languages::Languages;
use axum::body::Body;
use http_body_util::BodyExt;
use hyper::header::{self, HeaderValue};
use hyper::{Method, StatusCode};
use iroh::SecretKey;
use rmcp::model::{
    CallToolRequest, CallToolRequestParams, ClientCapabilities, ClientConfig, ClientRequest,
    Implementation,
};
use rmcp::service::{PeerRequestOptions, RoleClient, RunningService};
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
use crate::owner::OwnerStore;
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
  spec: '0.4'
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
    notes: Notes,
    _dir: tempfile::TempDir,
}

const NOTES_ID: &str = "app.example.notes";

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
    let profile = OwnerStore::for_test(dir.path(), &owner());
    let lifecycle = LifecycleConfig {
        public: public.clone(),
        data_dir: dir.path().to_owned(),
        apps_dir: dir.path().join("apps"),
        shares_dir: dir.path().join("shares"),
        core_container: None,
        tz: "UTC".into(),
        puid: owner_ids.uid(),
        pgid: owner_ids.gid(),
        install_timeout: Duration::from_secs(5),
        owner: "owner".into(),
        profile: profile.clone(),
        gateway_port: addr.port(),
        plugin_slice: None,
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
    let manifest = PluginPackage::parse(NOTES, &Languages::from_header(Some("zh-Hans")))
        .unwrap()
        .manifest;
    let id = manifest.id.clone();
    manager.register_for_test(manifest, PluginStatus::Running, None);
    let notes = Notes::default();
    manager.mcp_for_test(&id, serve_notes(notes.clone()).await);
    if let Some(permissions) = permissions {
        manager.register_source_for_test("com.example.agent", "127.0.0.0/8", permissions);
    }
    let cards = Arc::new(Cards::default());
    let confirmations = Arc::new(Confirmations::new(cards.clone(), Duration::from_secs(60)));
    let audit = dir.path().join("audit.jsonl");
    let bus = Bus::new(
        manager.clone(),
        confirmations,
        Arc::new(crate::mcp::audit::Audit::new(audit)),
    );
    let gateway = Gateway::new(
        public,
        profile,
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
        notes,
        _dir: dir,
    }
}

impl Home {
    fn mcp_url(&self) -> String {
        format!("http://{PLUGIN_MCP_HOST}:{}/mcp", self.addr.port())
    }

    /// 像 Agent 一样连上总线：`mcp.wonton.internal` 解析到网关。
    async fn agent(&self) -> RunningService<RoleClient, ClientConfig> {
        self.agent_with(ClientLifecycleMode::Auto {
            preferred_versions: vec![rmcp::model::ProtocolVersion::V_2026_07_28],
            legacy_version: Some(rmcp::model::ProtocolVersion::V_2025_11_25),
        })
        .await
    }

    /// `initialize` 握手的旧协议：有状态的会话。
    async fn legacy_agent(&self) -> RunningService<RoleClient, ClientConfig> {
        self.agent_with(ClientLifecycleMode::Initialize).await
    }

    async fn agent_with(
        &self,
        lifecycle: ClientLifecycleMode,
    ) -> RunningService<RoleClient, ClientConfig> {
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

    async fn pending(&self, headers: &[(&str, &str)]) -> (StatusCode, Value) {
        let mut request = hyper::Request::builder()
            .uri("/_wonton/api/confirmations")
            .header(header::HOST, format!("{CORE}:{}", self.addr.port()));
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        let request = request.body(Body::empty()).unwrap();
        let response = router(self.gateway.clone()).oneshot(request).await.unwrap();
        let status = response.status();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&body).unwrap_or(Value::Null))
    }

    /// 请求工具策略接口，主人令牌。
    async fn policy(&self, method: Method, path: &str, bearer: &str) -> (StatusCode, Value) {
        let request = hyper::Request::builder()
            .method(method)
            .uri(format!("/_wonton/api/plugins/{NOTES_ID}/tool-policy{path}"))
            .header(header::HOST, format!("{CORE}:{}", self.addr.port()))
            .header(header::AUTHORIZATION, format!("Bearer {bearer}"))
            .body(Body::empty())
            .unwrap();
        let response = router(self.gateway.clone()).oneshot(request).await.unwrap();
        let status = response.status();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&body).unwrap_or(Value::Null))
    }

    /// 请求“我来操作”的接口。
    async fn takeover(
        &self,
        method: Method,
        peer: Option<TunnelPeer>,
        headers: &[(&str, &str)],
    ) -> (StatusCode, Option<String>) {
        let mut request = hyper::Request::builder()
            .method(method)
            .uri(format!("/_wonton/api/plugins/{NOTES_ID}/takeover"))
            .header(header::HOST, format!("{CORE}:{}", self.addr.port()));
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        let mut request = request.body(Body::empty()).unwrap();
        if let Some(peer) = peer {
            request.extensions_mut().insert(peer);
        }
        let response = router(self.gateway.clone()).oneshot(request).await.unwrap();
        let kind = response
            .headers()
            .get(ERROR_HEADER)
            .map(|v| v.to_str().unwrap().to_owned());
        (response.status(), kind)
    }

    fn audit(&self) -> Vec<Value> {
        let text = fs::read_to_string(self._dir.path().join("audit.jsonl")).unwrap_or_default();
        text.lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
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

/// 让假插件一直不回答，再发一个只读调用，等它到了插件。
async fn hanging_call(
    home: &Home,
    agent: &RunningService<RoleClient, ClientConfig>,
) -> rmcp::service::RequestHandle<RoleClient> {
    home.notes.hang.store(true, Ordering::SeqCst);
    let request = CallToolRequestParams::new("app-example-notes__search");
    let handle = agent
        .peer()
        .send_request_with_option(
            ClientRequest::CallToolRequest(CallToolRequest::new(request)),
            PeerRequestOptions::no_options(),
        )
        .await
        .unwrap();
    // 第一次调用要连插件。
    tokio::time::sleep(Duration::from_millis(300)).await;
    handle
}

async fn plugin_told_to_cancel(home: &Home) {
    tokio::time::timeout(Duration::from_secs(10), async {
        while home.notes.cancelled.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("插件没有收到取消");
}

/// 走真实的 handler：Agent 取消，取消传到插件（插件规范的“工具调用的期限”）。
#[tokio::test]
async fn an_agent_cancelling_a_call_reaches_the_plugin_through_the_handler() {
    let home = start(Some(vec![Permission::Tools])).await;
    let agent = home.legacy_agent().await;
    let handle = hanging_call(&home, &agent).await;
    handle.cancel(Some("不要了".to_owned())).await.unwrap();
    plugin_told_to_cancel(&home).await;
}

/// Agent 关闭会话也一样。
#[tokio::test]
async fn an_agent_closing_its_session_cancels_what_it_was_running() {
    let home = start(Some(vec![Permission::Tools])).await;
    let agent = home.legacy_agent().await;
    let handle = hanging_call(&home, &agent).await;
    agent.cancel().await.unwrap();
    plugin_told_to_cancel(&home).await;
    drop(handle);
}

async fn post_from_plugin_network(
    home: &Home,
    path: &str,
) -> hyper::Response<hyper::body::Incoming> {
    post_body_from_plugin_network(home, path, "{}").await
}

async fn post_body_from_plugin_network(
    home: &Home,
    path: &str,
    body: &str,
) -> hyper::Response<hyper::body::Incoming> {
    let request = hyper::Request::builder()
        .method(Method::POST)
        .uri(format!("http://{}{path}", home.addr))
        .header(
            header::HOST,
            format!("{PLUGIN_MCP_HOST}:{}", home.addr.port()),
        )
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_owned()))
        .unwrap();
    HttpClient::new().send(request).await.unwrap()
}

const PERMISSION_REQUEST: &str = r#"{
    "tool_call_id": "call-1",
    "title": "运行 ls",
    "options": [
        {"choice_id": "once", "kind": "allow_once", "name": "这次允许"},
        {"choice_id": "always", "kind": "allow_always", "name": "总是允许"},
        {"choice_id": "no", "kind": "reject_once", "name": "拒绝"}
    ]
}"#;

/// 真的走 HTTP：Agent 从插件网络发长请求，主人经隧道回答，Agent 收到选的那一项（ADR-0035）。
#[tokio::test]
async fn the_agent_gets_the_owners_choice_over_http() {
    let home = Arc::new(start(Some(vec![Permission::Tools])).await);
    let path = wonton_protocol::AGENT_PERMISSION_PATH;
    let asked = {
        let home = home.clone();
        tokio::spawn(
            async move { post_body_from_plugin_network(&home, path, PERMISSION_REQUEST).await },
        )
    };
    let card = home.cards.wait_for(1).await;
    let answer_path = wonton_protocol::answer_path(&card.id);
    let device = home.paired_device();

    // 卡片上没有的选项，主人的回答也不算数，卡片还等着。
    let wrong = r#"{"answer": "approve", "choice_id": "nope"}"#;
    let (status, kind, _) = home.send(&answer_path, wrong, Some(device), &[]).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(kind.as_deref(), Some("invalid_input"));

    let right = r#"{"answer": "approve", "choice_id": "always"}"#;
    let (status, _, _) = home.send(&answer_path, right, Some(device), &[]).await;
    assert_eq!(status, StatusCode::OK);

    let response = asked.await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let outcome: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        outcome,
        json!({"outcome": "selected", "choice_id": "always"})
    );
}

#[tokio::test]
async fn a_bad_permission_request_gets_invalid_input() {
    let home = start(Some(vec![Permission::Tools])).await;
    let path = wonton_protocol::AGENT_PERMISSION_PATH;
    let unknown = PERMISSION_REQUEST.replace("allow_always", "allow_forever");
    for body in [unknown.as_str(), "不是 JSON", "{}"] {
        let response = post_body_from_plugin_network(&home, path, body).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(
            response.headers()[ERROR_HEADER],
            HeaderValue::from_static("invalid_input")
        );
    }
    assert!(home.cards.posted.lock().unwrap().is_empty());
}

/// 真的断开连接：卡片作废（ADR-0035）。靠 hyper 在对方断开时丢下请求的 future。
#[tokio::test]
async fn hanging_up_withdraws_the_card() {
    use tokio::io::AsyncWriteExt;

    let home = start(Some(vec![Permission::Tools])).await;
    let mut stream = tokio::net::TcpStream::connect(home.addr).await.unwrap();
    let head = format!(
        "POST {} HTTP/1.1\r\nHost: {PLUGIN_MCP_HOST}:{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
        wonton_protocol::AGENT_PERMISSION_PATH,
        home.addr.port(),
        PERMISSION_REQUEST.len(),
    );
    stream.write_all(head.as_bytes()).await.unwrap();
    stream
        .write_all(PERMISSION_REQUEST.as_bytes())
        .await
        .unwrap();
    let card = home.cards.wait_for(1).await;

    drop(stream);
    let withdrawn = |states: &[wonton_protocol::ConfirmationState]| {
        matches!(
            states.last(),
            Some(wonton_protocol::ConfirmationState::Failed { .. })
        )
    };
    for _ in 0..500 {
        if withdrawn(&home.cards.states(&card.id)) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(withdrawn(&home.cards.states(&card.id)));
}

/// Agent 权限确认的入口和 MCP 总线同一道门（ADR-0035）。
#[tokio::test]
async fn the_agent_permission_entry_is_behind_the_same_gate() {
    let path = wonton_protocol::AGENT_PERMISSION_PATH;
    let response = post_from_plugin_network(&start(None).await, path).await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let without = start(Some(vec![Permission::Models])).await;
    let response = post_from_plugin_network(&without, path).await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let with = start(Some(vec![Permission::Tools])).await;
    let response = post_from_plugin_network(&with, path).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        response.headers()[ERROR_HEADER],
        HeaderValue::from_static("invalid_input")
    );
}

#[tokio::test]
async fn only_plugin_networks_with_the_tools_permission_get_in() {
    async fn probe(home: &Home) -> hyper::Response<hyper::body::Incoming> {
        post_from_plugin_network(home, "/mcp").await
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

/// MCP 总线的文字按主人的语言写（ADR-0043）：Agent 的调用没有请求头，用 owner_languages；
/// 期望值由目录格式化出来比，ja 和 en 要真的不同。
#[tokio::test]
async fn mcp_messages_follow_the_owner_languages() {
    use crate::l10n::CATALOG;

    let home = start(Some(vec![Permission::Models])).await;
    home.gateway.plugins.note_owner_languages(Some("ja"));
    let response = post_from_plugin_network(&home, "/mcp").await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let reply: Value = serde_json::from_slice(&body).unwrap();
    // 权限检查报的是调用方（Agent 的插件）的编号，不是被调用的应用。
    let args = &[("id", "com.example.agent".into())];
    let expected =
        |tag: &str| CATALOG.format(CATALOG.negotiate(&[tag]), "gateway-tools-permission", args);
    assert_eq!(reply["error"]["message"], expected("ja"));
    assert_ne!(expected("ja"), expected("en"));
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
    // 比核心新的 App 发来不认识的回答：不算同意，卡片还等着。
    let newer = r#"{"answer": "allow_forever"}"#;
    let (status, kind, _) = home.send(&path, newer, Some(device), &[]).await;
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

/// 开发环境没有配对设备：主人令牌能看到等回答的卡片，也能回答（ADR-0024）。
#[tokio::test]
async fn the_owner_token_lists_and_answers_cards() {
    let home = start(Some(vec![Permission::Tools])).await;
    let bearer = format!("Bearer {TOKEN}");
    let owner = [("authorization", bearer.as_str())];
    let (status, _) = home.pending(&[]).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, list) = home.pending(&owner).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list["confirmations"], json!([]));

    let agent = home.agent().await;
    let peer = agent.peer().clone();
    let call = tokio::spawn(async move {
        let request = CallToolRequestParams::new("app-example-notes__delete")
            .with_arguments(json!({"name": "y"}).as_object().cloned().unwrap());
        peer.call_tool(request).await.unwrap()
    });
    let card = home.cards.wait_for(1).await;
    let (_, list) = home.pending(&owner).await;
    assert_eq!(list["confirmations"][0]["id"], card.id.as_str());

    let path = wonton_protocol::answer_path(&card.id);
    let approve = r#"{"answer": "approve"}"#;
    let wrong = [("authorization", "Bearer wrong")];
    let (status, _, _) = home.send(&path, approve, None, &wrong).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _, reply) = home.send(&path, approve, None, &owner).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(reply["state"]["type"], "running");
    assert_eq!(text(&call.await.unwrap()), "删掉了 y.txt");
    let (_, list) = home.pending(&owner).await;
    assert_eq!(list["confirmations"], json!([]));
}

/// 主人经接口放宽，下一次调用就不发卡片；没有身份的请求改不了（ADR-0024）。
#[tokio::test]
async fn the_owner_marks_a_tool_read_only_through_the_api() {
    let home = start(Some(vec![Permission::Tools])).await;
    let (status, _) = home.policy(Method::GET, "", "wrong").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, view) = home.policy(Method::GET, "", TOKEN).await;
    assert_eq!(status, StatusCode::OK);
    let access = |view: &Value, tool: &str| {
        let tools = view["tools"].as_array().unwrap();
        let tool = tools.iter().find(|t| t["name"] == tool).unwrap();
        tool["access"].as_str().unwrap().to_owned()
    };
    assert_eq!(access(&view, "delete"), "sensitive");
    assert_eq!(access(&view, "search"), "plugin_read_only");

    let (status, _) = home.policy(Method::PUT, "/read-only/nope", TOKEN).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = home
        .policy(Method::PUT, "/always/com.example.nobody", TOKEN)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, view) = home.policy(Method::PUT, "/read-only/delete", TOKEN).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(access(&view, "delete"), "owner_read_only");

    let agent = home.agent().await;
    let request = CallToolRequestParams::new("app-example-notes__delete")
        .with_arguments(json!({"name": "y"}).as_object().cloned().unwrap());
    let result = agent.peer().call_tool(request).await.unwrap();
    assert_eq!(text(&result), "删掉了 y.txt");
    assert_eq!(home.cards.posted.lock().unwrap().len(), 0);

    let (_, view) = home
        .policy(Method::DELETE, "/read-only/delete", TOKEN)
        .await;
    assert_eq!(access(&view, "delete"), "sensitive");
}

/// 卡片上点“都允许”，之后不发卡片，设置里看得到；停止后回到发卡片（ADR-0024）。
#[tokio::test]
async fn the_owner_opens_and_stops_a_window_through_the_api() {
    let home = start(Some(vec![Permission::Tools])).await;
    let bearer = format!("Bearer {TOKEN}");
    let owner = [("authorization", bearer.as_str())];
    let agent = home.agent().await;
    let call = |name: &str| {
        let peer = agent.peer().clone();
        let request = CallToolRequestParams::new("app-example-notes__delete")
            .with_arguments(json!({"name": name}).as_object().cloned().unwrap());
        tokio::spawn(async move { peer.call_tool(request).await.unwrap() })
    };

    let first = call("first");
    let card = home.cards.wait_for(1).await;
    let path = wonton_protocol::answer_path(&card.id);
    let allow = r#"{"answer": "allow_window"}"#;
    let (status, _, reply) = home.send(&path, allow, None, &owner).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(reply["state"]["type"], "running");
    assert_eq!(text(&first.await.unwrap()), "删掉了 first.txt");
    assert_eq!(text(&call("second").await.unwrap()), "删掉了 second.txt");
    assert_eq!(home.cards.posted.lock().unwrap().len(), 1);

    let (_, view) = home.policy(Method::GET, "", TOKEN).await;
    assert_eq!(view["windows"][0]["card"], card.id.as_str());
    assert_eq!(view["windows"][0]["caller"], "com.example.agent");

    let stop = |bearer: String| {
        let request = hyper::Request::builder()
            .method(Method::DELETE)
            .uri(wonton_protocol::window_path(&card.id))
            .header(header::HOST, format!("{CORE}:{}", home.addr.port()))
            .header(header::AUTHORIZATION, format!("Bearer {bearer}"))
            .body(Body::empty())
            .unwrap();
        router(home.gateway.clone()).oneshot(request)
    };
    let status = stop("wrong".to_owned()).await.unwrap().status();
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let status = stop(TOKEN.to_owned()).await.unwrap().status();
    assert_eq!(status, StatusCode::NO_CONTENT);
    let status = stop(TOKEN.to_owned()).await.unwrap().status();
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (_, view) = home.policy(Method::GET, "", TOKEN).await;
    assert_eq!(view["windows"], json!([]));

    let third = call("third");
    let card = home.cards.wait_for(2).await;
    let path = wonton_protocol::answer_path(&card.id);
    home.send(&path, r#"{"answer": "reject"}"#, None, &owner)
        .await;
    third.await.unwrap();
}

#[tokio::test]
async fn only_the_owner_can_take_over_and_a_revoked_device_stops_operating() {
    let home = start(None).await;
    let (status, _) = home.takeover(Method::PUT, None, &[]).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "没有身份");
    let peer = home.paired_device();
    let from_page = [("sec-fetch-site", "same-site")];
    let (status, _) = home.takeover(Method::PUT, Some(peer), &from_page).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "插件网页经 WebView 发不了");
    assert!(home.audit().is_empty());

    let (status, _) = home.takeover(Method::PUT, Some(peer), &[]).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let started = home.audit();
    assert_eq!(started[0]["takeover"], "started");
    assert_eq!(started[0]["by"], format!("device:{}", peer.0));

    let bearer = format!("Bearer {TOKEN}");
    let request = hyper::Request::builder()
        .method(Method::DELETE)
        .uri(format!("/_wonton/api/devices/{}", peer.0))
        .header(header::HOST, format!("{CORE}:{}", home.addr.port()))
        .header(header::AUTHORIZATION, &bearer)
        .body(Body::empty())
        .unwrap();
    let response = router(home.gateway.clone()).oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let ended = home.audit();
    assert_eq!(ended[1]["takeover"], "ended");
    assert_eq!(ended[1]["reason"], "device_revoked");
}

#[tokio::test]
async fn a_plugin_that_is_not_running_cannot_be_taken_over() {
    let home = start(None).await;
    let manifest = PluginPackage::parse(NOTES, &Languages::from_header(Some("zh-Hans")))
        .unwrap()
        .manifest;
    home.gateway.plugins.register_for_test(
        manifest,
        PluginStatus::Degraded {
            reason: "停了".to_owned(),
        },
        None,
    );
    let bearer = format!("Bearer {TOKEN}");
    let auth = [("authorization", bearer.as_str())];
    let (status, kind) = home.takeover(Method::PUT, None, &auth).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(kind.as_deref(), Some("not_ready"));
    let (status, _) = home.takeover(Method::DELETE, None, &auth).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "交还可以重复发");
}
