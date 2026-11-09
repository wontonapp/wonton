//! `mcp.headers` 解析好的请求头：发往上游的每个请求都带上；collect 的密钥换了以后重连并用新值；上游回 401
//! 时不自动重试、不重新取密钥，错误说明是插件的 MCP 拒绝了核心的凭据。

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};

use super::upstream::Upstreams;
use crate::plugin::{McpHeaders, McpServer, PluginId};

fn server_with(url: String, token: &str) -> McpServer {
    McpServer {
        id: PluginId::parse("app.example.notes").unwrap(),
        title: Default::default(),
        url,
        socket: None,
        headers: McpHeaders::from_pairs([("authorization", format!("Bearer {token}"))]),
        tool_limits: Default::default(),
        tools: None,
    }
}

async fn with_token(
    expected: &Arc<Mutex<Option<String>>>,
    seen: &Arc<Mutex<Vec<String>>>,
    initializes: &Arc<AtomicUsize>,
    headers: axum::http::HeaderMap,
    body: String,
) -> Response {
    let authorization = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    seen.lock()
        .unwrap()
        .push(authorization.clone().unwrap_or_default());
    if let Some(token) = expected.lock().unwrap().as_ref()
        && authorization.as_deref() != Some(&format!("Bearer {token}"))
    {
        return hyper::StatusCode::UNAUTHORIZED.into_response();
    }
    let message: Value = serde_json::from_str(&body).unwrap();
    let reply = |result: Value| {
        axum::Json(json!({"jsonrpc": "2.0", "id": message["id"], "result": result})).into_response()
    };
    match message["method"].as_str().unwrap_or_default() {
        "initialize" => {
            initializes.fetch_add(1, Ordering::SeqCst);
            reply(json!({
                "protocolVersion": "2025-11-25",
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "auth", "version": "1"},
            }))
        }
        "tools/list" => reply(json!({"tools": [{"name": "a", "inputSchema": {"type": "object"}}]})),
        "tools/call" => reply(json!({"content": [{"type": "text", "text": "好"}]})),
        _ => hyper::StatusCode::ACCEPTED.into_response(),
    }
}

/// 只认 `Authorization: Bearer <expected>` 的假插件，每个请求的 `Authorization` 都记下来。
struct AuthPlugin {
    server: McpServer,
    expected: Arc<Mutex<Option<String>>>,
    seen: Arc<Mutex<Vec<String>>>,
    initializes: Arc<AtomicUsize>,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    serving: tokio::task::JoinHandle<()>,
}

impl Drop for AuthPlugin {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        self.serving.abort();
    }
}

async fn auth_plugin() -> AuthPlugin {
    let expected = Arc::new(Mutex::new(None));
    let seen = Arc::new(Mutex::new(Vec::new()));
    let initializes = Arc::new(AtomicUsize::new(0));
    let (expected_handler, seen_handler, initializes_handler) =
        (expected.clone(), seen.clone(), initializes.clone());
    let app = axum::Router::new().route(
        "/mcp",
        axum::routing::post(move |headers: axum::http::HeaderMap, body: String| {
            let expected = expected_handler.clone();
            let seen = seen_handler.clone();
            let initializes = initializes_handler.clone();
            async move { with_token(&expected, &seen, &initializes, headers, body).await }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/mcp", listener.local_addr().unwrap());
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let serving = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = stopped.await;
            })
            .await
            .unwrap();
    });
    AuthPlugin {
        server: server_with(url, ""),
        expected,
        seen,
        initializes,
        stop: Some(stop),
        serving,
    }
}

impl AuthPlugin {
    fn require(&self, token: &str) {
        *self.expected.lock().unwrap() = Some(token.to_owned());
    }

    fn server_with(&self, token: &str) -> McpServer {
        server_with(self.server.url.clone(), token)
    }
}

/// 清单 `mcp.headers` 解析好的头：握手、列工具、调用的每个请求都带上。
#[tokio::test(flavor = "multi_thread")]
async fn declared_headers_go_on_every_request() {
    let upstreams = Upstreams::new();
    let plugin = auth_plugin().await;
    plugin.require("s3cret");
    let server = plugin.server_with("s3cret");
    let listed = upstreams.tools(&server).await;
    assert!(listed.is_ok(), "{listed:?}");
    let call = tokio::time::timeout(
        Duration::from_secs(20),
        upstreams.call(&server, "a", None, None, std::future::pending()),
    )
    .await
    .expect("到期了还没有结果");
    assert!(call.is_ok(), "{call:?}");
    let seen = plugin.seen.lock().unwrap().clone();
    assert!(!seen.is_empty(), "假插件一个请求都没收到");
    assert!(
        seen.iter().all(|value| value == "Bearer s3cret"),
        "{seen:?}"
    );
}

/// collect 的密钥换了（插件条目带着新头回来）：总线的 retain 和下一次连接都认新的值。
#[tokio::test]
async fn a_changed_secret_reconnects_and_uses_the_new_value() {
    let upstreams = Upstreams::new();
    let plugin = auth_plugin().await;
    plugin.require("old");
    assert!(upstreams.tools(&plugin.server_with("old")).await.is_ok());
    assert_eq!(plugin.initializes.load(Ordering::SeqCst), 1);

    // 上游换了令牌，核心重新收存后条目带着新头回来；retain 先把旧连接丢掉。
    plugin.require("new");
    let renewed = plugin.server_with("new");
    upstreams.retain(std::slice::from_ref(&renewed));
    let listed = upstreams.tools(&renewed).await;
    assert!(listed.is_ok(), "{listed:?}");
    assert_eq!(plugin.initializes.load(Ordering::SeqCst), 2);
    assert_eq!(plugin.seen.lock().unwrap().last().unwrap(), "Bearer new");

    // 不经 retain 也一样：槽里存的头对不上现在的值，重连并用新值握手。
    plugin.require("again");
    let again = plugin.server_with("again");
    let listed = upstreams.tools(&again).await;
    assert!(listed.is_ok(), "{listed:?}");
    assert_eq!(plugin.initializes.load(Ordering::SeqCst), 3);
    assert_eq!(plugin.seen.lock().unwrap().last().unwrap(), "Bearer again");
}

/// 上游回 401：不重试、不重新取密钥，错误说明是插件的 MCP 拒绝了核心的凭据。
#[tokio::test]
async fn a_401_from_the_upstream_says_the_plugin_rejected_the_credentials() {
    let upstreams = Upstreams::new();
    let plugin = auth_plugin().await;
    plugin.require("s3cret");
    let error = upstreams.tools(&plugin.server).await.unwrap_err();
    assert!(error.contains("rejected the credentials"), "{error}");
}
