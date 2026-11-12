//! 插件停了以后调用不复用池子里的空闲连接：往那条“还开着”的连接上发的请求分不清发没发出，
//! 会把“没发出”报成“不确定”。替身把两条路都做成确定的：停了以后新连接一律被拒，
//! 已建立的连接不主动关，再收到任何字节就直接关、不回答。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use rmcp::model::JsonObject;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use super::upstream::{CallError, Upstreams};
use crate::plugin::{McpHeaders, McpServer, PluginId};

/// 回归时测试要失败，不能卡住。
const WITHIN: Duration = Duration::from_secs(20);

const NO_ARGUMENTS: Option<JsonObject> = None;

async fn within<T>(future: impl Future<Output = T>) -> T {
    tokio::time::timeout(WITHIN, future)
        .await
        .expect("到期了还没有结果")
}

/// 先正常服务 `tools()`（回答带 content-length，连接保持打开），停了以后：新连接一律拒绝，
/// 已建立的连接不主动关，再收到任何字节就直接关。这样“复用旧连接”和“新建连接”走的是哪条路，
/// 看错误就能确定，不靠时序碰运气。
async fn stopped_plugin() -> Stopped {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let closed = Arc::new(AtomicBool::new(false));
    let (closing, mut closed_signal) = tokio::sync::oneshot::channel();
    let serving = {
        let closed = closed.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let (socket, _) = accepted.unwrap();
                        tokio::spawn(serve_connection(socket, closed.clone()));
                    }
                    _ = &mut closed_signal => break,
                }
            }
            // 先停监听（新连接被拒），再打开关闭开关（旧连接收到字节就关）。
            drop(listener);
            closed.store(true, Ordering::SeqCst);
        })
    };
    Stopped {
        server: McpServer {
            id: PluginId::parse("app.example.stopped").unwrap(),
            title: Default::default(),
            url: format!("http://{address}/mcp"),
            socket: None,
            headers: McpHeaders::default(),
            tool_limits: Default::default(),
            tools: None,
        },
        serving,
        closing,
    }
}

struct Stopped {
    server: McpServer,
    serving: tokio::task::JoinHandle<()>,
    closing: tokio::sync::oneshot::Sender<()>,
}

impl Stopped {
    async fn stop(self) -> McpServer {
        self.closing.send(()).unwrap();
        self.serving.await.unwrap();
        self.server
    }
}

async fn serve_connection(mut socket: TcpStream, closed: Arc<AtomicBool>) {
    let mut pending: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let read = socket.read(&mut chunk).await.unwrap();
        if read == 0 {
            // 客户端关了连接（不复用空闲连接时，每次回答完就关）。
            return;
        }
        if closed.load(Ordering::SeqCst) {
            // 停了以后再收到任何字节：发上来的是往这条连接上发的请求，直接关，不回答。
            return;
        }
        pending.extend_from_slice(&chunk[..read]);
        while let Some((method, id, consumed)) = parsed_request(&pending) {
            pending.drain(..consumed);
            socket.write_all(&answer(&method, id)).await.unwrap();
        }
    }
}

/// 一个完整的请求：方法、id（通知没有）和它的字节数。没凑齐就给 None。
fn parsed_request(buffer: &[u8]) -> Option<(String, Option<Value>, usize)> {
    let end = buffer.windows(4).position(|window| window == b"\r\n\r\n")?;
    let head = std::str::from_utf8(&buffer[..end]).ok()?;
    let mut length = 0usize;
    for line in head.lines().skip(1) {
        if let Some((name, value)) = line.split_once(':')
            && name.trim().eq_ignore_ascii_case("content-length")
        {
            length = value.trim().parse().unwrap_or(0);
        }
    }
    let total = end + 4 + length;
    if buffer.len() < total {
        return None;
    }
    let message: Value = serde_json::from_slice(&buffer[end + 4..total]).ok()?;
    let method = message["method"].as_str()?.to_owned();
    let id = (!message["id"].is_null()).then(|| message["id"].clone());
    Some((method, id, total))
}

fn answer(method: &str, id: Option<Value>) -> Vec<u8> {
    let body = match (method, id) {
        ("initialize", Some(id)) => json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "protocolVersion": "2025-11-25",
                "capabilities": {"resources": {}, "tools": {}},
                "serverInfo": {"name": "stopped", "version": "1"},
            },
        }),
        ("tools/list", Some(id)) => json!({"jsonrpc": "2.0", "id": id, "result": {"tools": []}}),
        (_, None) => return b"HTTP/1.1 202 Accepted\r\ncontent-length: 0\r\n\r\n".to_vec(),
        (_, Some(id)) => json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {"code": -32601, "message": "替身不回答这个方法"},
        }),
    };
    let body = body.to_string();
    format!(
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{}",
        body.len(),
        body
    )
    .into_bytes()
}

/// 插件停了以后，`call` 必须新建连接（池子里的空闲连接留着发请求，分不清发没发出）：
/// 这里的新连接一律被拒，所以结果必须是“没发出去”，不是“不确定”。
#[tokio::test]
async fn a_call_to_a_stopped_plugin_does_not_reuse_the_stale_connection() {
    let plugin = stopped_plugin().await;
    let upstreams = Upstreams::new();
    upstreams.tools(&plugin.server).await.unwrap();
    let server = plugin.stop().await;
    let called =
        within(upstreams.call(&server, "tool", NO_ARGUMENTS, None, std::future::pending())).await;
    assert!(matches!(called, Err(CallError::NotSent(_))), "{called:?}");
}
