//! 总线连插件测试用的假插件（[`super::upstream_tests`]）：一个 axum 服务按资源的地址、工具的名字和脚本的状态决定怎么回，
//! 超限、断开、不回答、翻页这些行为都能演。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use axum::response::{IntoResponse, Response};
use rmcp::model::JsonObject;
use serde_json::{Value, json};

use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::plugin::{McpServer, PluginId};

pub(super) const SLOW: Duration = Duration::from_millis(200);

pub(super) fn result(id: &Value, result: Value) -> Response {
    axum::Json(json!({"jsonrpc": "2.0", "id": id, "result": result})).into_response()
}

pub(super) fn endless(content_type: &'static str) -> Response {
    let chunks = futures_util::stream::repeat_with(|| {
        Ok::<_, std::convert::Infallible>(axum::body::Bytes::from(vec![b'x'; 1 << 16]))
    });
    let body = axum::body::Body::from_stream(chunks);
    ([(hyper::header::CONTENT_TYPE, content_type)], body).into_response()
}

/// 响应体写到一半连接断了。
fn cut(content_type: &'static str) -> Response {
    let chunks = futures_util::stream::iter([
        Ok(axum::body::Bytes::from_static(b"data: {\"jsonrpc")),
        Err(std::io::Error::other("断了")),
    ]);
    let body = axum::body::Body::from_stream(chunks);
    ([(hyper::header::CONTENT_TYPE, content_type)], body).into_response()
}

fn text(uri: &str) -> Value {
    json!({"contents": [{"uri": uri, "text": "好"}]})
}

fn tool_text() -> Value {
    json!({"content": [{"type": "text", "text": "好"}]})
}

async fn read(
    id: &Value,
    uri: &str,
    content_type: &'static str,
    held: &tokio::sync::Notify,
) -> Response {
    match uri {
        "x://big" => endless(content_type),
        "x://refused" => axum::Json(json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {"code": -32002, "message": "没有这个资源"},
        }))
        .into_response(),
        "x://http500" => hyper::StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        "x://wordy500" => (
            hyper::StatusCode::INTERNAL_SERVER_ERROR,
            "磁盘坏了".repeat(1 << 16),
        )
            .into_response(),
        "x://huge500" => {
            let mut response = endless("text/plain");
            *response.status_mut() = hyper::StatusCode::INTERNAL_SERVER_ERROR;
            response
        }
        // supergateway 换会话后这样回。
        "x://null-id" => (
            hyper::StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32000, "message": "会话没了"}})),
        )
            .into_response(),
        "x://wrong-error" => axum::Json(json!({
            "jsonrpc": "2.0",
            "id": 987654,
            "error": {"code": -32000, "message": "不是你的"},
        }))
        .into_response(),
        "x://wrong-result" => result(&json!(987654), text(uri)),
        "x://html" => ([(hyper::header::CONTENT_TYPE, "text/html")], "<p>登录</p>").into_response(),
        "x://garbled" => (
            [(hyper::header::CONTENT_TYPE, "application/json")],
            "{ 不是",
        )
            .into_response(),
        "x://slowcut" => {
            tokio::time::sleep(SLOW).await;
            cut(content_type)
        }
        "x://cut" => cut(content_type),
        "x://slow" => {
            tokio::time::sleep(SLOW).await;
            result(id, text(uri))
        }
        "x://held" => {
            held.notified().await;
            result(id, text(uri))
        }
        _ => result(id, text(uri)),
    }
}

/// `tools/list` 怎么回。
#[derive(Clone, Copy, Default)]
pub(super) enum Lists {
    #[default]
    Empty,
    /// 每一页都不大，加起来超过上限。
    Heavy,
    /// 一直有下一页。
    Endless,
    /// 下一页的 cursor 总是同一个。
    SameCursor,
    /// 一页就有太多工具。
    TooMany,
}

fn tool_named(name: String) -> Value {
    json!({"name": name, "inputSchema": {"type": "object"}})
}

fn list(id: &Value, lists: Lists, cursor: &Value) -> Response {
    let tools = || vec![tool_named("a".to_owned())];
    match lists {
        Lists::Empty => result(id, json!({"tools": []})),
        Lists::Endless => {
            let next = cursor.as_str().unwrap_or_default().len() + 1;
            result(
                id,
                json!({"tools": tools(), "nextCursor": "x".repeat(next)}),
            )
        }
        Lists::SameCursor => result(id, json!({"tools": tools(), "nextCursor": "again"})),
        Lists::Heavy => {
            let next = cursor.as_str().unwrap_or_default().len() + 1;
            let mut heavy = tool_named("heavy".to_owned());
            heavy["description"] = json!("x".repeat(1 << 20));
            result(
                id,
                json!({"tools": [heavy], "nextCursor": "x".repeat(next)}),
            )
        }
        Lists::TooMany => {
            let tools: Vec<_> = (0..3_000).map(|n| tool_named(n.to_string())).collect();
            result(id, json!({"tools": tools}))
        }
    }
}

/// 进度事件，最后是结果。`steps` 为空时一直发进度。
fn with_progress(id: &Value, token: Value, steps: Option<usize>) -> Response {
    let event = |message: Value| {
        Ok::<_, std::convert::Infallible>(axum::body::Bytes::from(format!("data: {message}\n\n")))
    };
    let progress = {
        let token = token.clone();
        move |step: usize| {
            json!({
                "jsonrpc": "2.0",
                "method": "notifications/progress",
                "params": {"progressToken": token.clone(), "progress": step},
            })
        }
    };
    let id = id.clone();
    let chunks = futures_util::stream::unfold(0usize, move |step| {
        let (progress, id) = (progress.clone(), id.clone());
        async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            if steps.is_some_and(|steps| step > steps) {
                return None;
            }
            let message = if steps == Some(step) {
                json!({"jsonrpc": "2.0", "id": id, "result": tool_text()})
            } else {
                progress(step)
            };
            Some((event(message), step + 1))
        }
    });
    let body = axum::body::Body::from_stream(chunks);
    ([(hyper::header::CONTENT_TYPE, "text/event-stream")], body).into_response()
}

async fn answer(state: Scripted, content_type: &'static str, body: String) -> Response {
    let message: Value = serde_json::from_str(&body).unwrap();
    let id = if state.string_ids.load(Ordering::SeqCst) {
        &json!(message["id"].to_string())
    } else {
        &message["id"]
    };
    let params = &message["params"];
    match message["method"].as_str().unwrap_or_default() {
        "initialize" => {
            state.initializes.fetch_add(1, Ordering::SeqCst);
            result(
                id,
                json!({
                    "protocolVersion": "2025-11-25",
                    "capabilities": {"resources": {}, "tools": {}},
                    "serverInfo": {"name": "scripted", "version": "1"},
                }),
            )
        }
        "resources/read" => {
            let uri = params["uri"].as_str().unwrap();
            read(id, uri, content_type, &state.held_reads).await
        }
        "tools/call" if params["name"] == "big" => endless(content_type),
        "tools/call" if params["name"] == "silent" => std::future::pending().await,
        "tools/call" if params["name"] == "progress" || params["name"] == "chatty" => {
            let steps = (params["name"] == "progress").then_some(8);
            with_progress(id, params["_meta"]["progressToken"].clone(), steps)
        }
        "tools/call" => {
            tokio::time::sleep(SLOW).await;
            result(id, tool_text())
        }
        "tools/list" => {
            // 还有几次不回答：会话卡死的样子，TCP 没断。
            let left = state
                .stuck_lists
                .try_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1));
            if left.is_ok() {
                std::future::pending::<()>().await;
            }
            let lists = *state.lists.lock().unwrap();
            list(id, lists, &params["cursor"])
        }
        "notifications/cancelled" => {
            state.cancelled.fetch_add(1, Ordering::SeqCst);
            hyper::StatusCode::ACCEPTED.into_response()
        }
        _ => hyper::StatusCode::ACCEPTED.into_response(),
    }
}

/// 假插件的状态。
#[derive(Clone, Default)]
pub(super) struct Scripted {
    pub(super) initializes: Arc<AtomicUsize>,
    pub(super) stuck_lists: Arc<AtomicUsize>,
    /// 收到了几次 `notifications/cancelled`。
    pub(super) cancelled: Arc<AtomicUsize>,
    pub(super) lists: Arc<std::sync::Mutex<Lists>>,
    /// 把数字 id 回成字符串。
    pub(super) string_ids: Arc<AtomicBool>,
    /// `x://held` 等到这里放行才回答。
    pub(super) held_reads: Arc<tokio::sync::Notify>,
}

/// 假插件，按资源的地址或工具的名字决定怎么回。超限和断开的响应用 `content_type`，其余都是普通 JSON。
pub(super) struct Plugin {
    pub(super) server: McpServer,
    pub(super) state: Scripted,
    pub(super) initializes: Arc<AtomicUsize>,
    pub(super) stuck_lists: Arc<AtomicUsize>,
    stop: tokio::sync::oneshot::Sender<()>,
    serving: tokio::task::JoinHandle<()>,
}

impl Plugin {
    /// 监听停了，空闲的连接也关了：和容器重启一样。
    pub(super) async fn stop(self) -> McpServer {
        self.stop.send(()).unwrap();
        self.serving.await.unwrap();
        self.server
    }
}

pub(super) async fn plugin(content_type: &'static str) -> Plugin {
    let state = Scripted::default();
    let served = state.clone();
    let app = axum::Router::new().route(
        "/mcp",
        axum::routing::post(move |body: String| answer(served.clone(), content_type, body)),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let serving = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async { drop(stopped.await) })
            .await
            .unwrap();
    });
    Plugin {
        stop,
        serving,
        server: server_at(format!("http://{address}/mcp")),
        initializes: state.initializes.clone(),
        stuck_lists: state.stuck_lists.clone(),
        state,
    }
}

pub(super) fn server_at(url: String) -> McpServer {
    McpServer {
        id: PluginId::parse("app.example.notes").unwrap(),
        title: Default::default(),
        url,
        socket: None,
        headers: Default::default(),
        tool_limits: Default::default(),
        tools: None,
    }
}

/// 声明了一个读不下的大小的假插件，手写 HTTP 回答（axum/hyper 不肯声明得比响应体大）：initialize 照常答，
/// `declared` 工具只发一个声明了 100 MiB 的响应头就关连接，谁也不该去读这个响应体。
pub(super) async fn declared_plugin() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            tokio::spawn(serve_declared(socket));
        }
    });
    format!("http://{address}/mcp")
}

async fn serve_declared(mut socket: tokio::net::TcpStream) {
    let mut buf: Vec<u8> = Vec::new();
    let head_end = loop {
        if socket.read_buf(&mut buf).await.unwrap_or(0) == 0 {
            return;
        }
        if let Some(head_end) = buf.windows(4).position(|w| w == b"\r\n\r\n").map(|p| p + 4) {
            break head_end;
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
    let body_len: usize = head
        .lines()
        .find_map(|line| {
            line.to_ascii_lowercase()
                .strip_prefix("content-length:")
                .and_then(|value| value.trim().parse().ok())
        })
        .unwrap_or(0);
    while buf.len() < head_end + body_len {
        if socket.read_buf(&mut buf).await.unwrap_or(0) == 0 {
            return;
        }
    }
    let body = String::from_utf8_lossy(&buf[head_end..]).into_owned();
    let reply = if body.contains("\"method\":\"initialize\"") {
        http(&format!(
            "{{\"jsonrpc\":\"2.0\",\"id\":{},\"result\":{{\"protocolVersion\":\"2025-11-25\",\"capabilities\":{{}},\"serverInfo\":{{\"name\":\"declared\",\"version\":\"1\"}}}}}}",
            json_id(&body)
        ))
    } else if body.contains("notifications/") {
        "HTTP/1.1 202 Accepted\r\ncontent-length: 0\r\nconnection: close\r\n\r\n".to_owned()
    } else if body.contains("\"declared\"") {
        format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
            100u64 << 20
        )
    } else if body.contains("\"jsonrpc\"") {
        http(&format!(
            "{{\"jsonrpc\":\"2.0\",\"id\":{},\"result\":{{\"content\":[{{\"type\":\"text\",\"text\":\"好\"}}]}}}}",
            json_id(&body)
        ))
    } else {
        "HTTP/1.1 405 Method Not Allowed\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
            .to_owned()
    };
    let _ = socket.write_all(reply.as_bytes()).await;
}

fn http(json: &str) -> String {
    format!(
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{json}",
        json.len()
    )
}

/// 从请求体里把 JSON-RPC 的 id 原样抠出来回过去。
fn json_id(body: &str) -> &str {
    let rest = &body[body.find("\"id\":").map_or(0, |at| at + 5)..];
    let end = rest.find([',', '}']).unwrap_or(rest.len());
    rest[..end].trim()
}

pub(super) const NO_ARGUMENTS: Option<JsonObject> = None;
