//! 使用方脚本（ADR-0061 第 1 节的契约测试一侧）：同一段脚本对最小提供方直连、和经核心转发各跑一遍。
//! 握手是手写的：核心按 Host 头路由，测试要能指定 `agent_acp.wonton.internal` 而连到本机端口。

use axum::http::header::{CONNECTION, HOST, SEC_WEBSOCKET_KEY, UPGRADE};
use axum::http::{Method, StatusCode};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use futures_util::{SinkExt, StreamExt};
use hyper_util::rt::TokioIo;
use serde_json::{Value, json};
use tokio::net::TcpStream;
use tokio_websockets::{ClientBuilder, Message, WebSocketStream};

/// 升级的结果：状态码，101 时带一条可用的连接。
pub(super) struct Upgrade {
    pub(super) status: StatusCode,
    pub(super) ws: Option<WebSocketStream<TokioIo<hyper::upgrade::Upgraded>>>,
}

/// 连 `gateway`（核心或直连提供方），Host 头写 `host`，带 `headers` 里的其余头。
pub(super) async fn connect(
    gateway: std::net::SocketAddr,
    host: &str,
    headers: &[(&str, &str)],
) -> Upgrade {
    let stream = TcpStream::connect(gateway).await.unwrap();
    let mut key = [0u8; 16];
    getrandom::fill(&mut key).expect("操作系统随机数源不可用");
    let mut builder = axum::http::Request::builder()
        .method(Method::GET)
        .uri(format!("http://{host}/acp"))
        .header(CONNECTION, "Upgrade")
        .header(UPGRADE, "websocket")
        .header("sec-websocket-version", "13")
        .header(SEC_WEBSOCKET_KEY, STANDARD.encode(key));
    builder = builder.header(HOST, host);
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    let request = builder.body(axum::body::Body::empty()).unwrap();
    let (mut sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
        .await
        .unwrap();
    // 101 的升级要连接任务显式支持：不带 with_upgrades 会在应答时报 ManualUpgrade。
    tokio::spawn(conn.with_upgrades());
    let response = sender.send_request(request).await.unwrap();
    let status = response.status();
    if status != StatusCode::SWITCHING_PROTOCOLS {
        // 非 101：把核心的错误体带出来，失败信息里能直接看到原因。
        let body = http_body_util::BodyExt::collect(response.into_body())
            .await
            .map(|b| b.to_bytes())
            .map(|b| String::from_utf8_lossy(&b).to_string())
            .unwrap_or_default();
        eprintln!("升级被拒 {status}: {body}");
        return Upgrade { status, ws: None };
    }
    let upgraded = hyper::upgrade::on(response).await.unwrap();
    Upgrade {
        status,
        ws: Some(ClientBuilder::new().take_over(TokioIo::new(upgraded))),
    }
}

/// 建连后的收发：脚本里的步骤按这里的一对对助手走。
pub(super) struct Ws(pub(super) WebSocketStream<TokioIo<hyper::upgrade::Upgraded>>);

impl Ws {
    pub(super) async fn send(&mut self, value: Value) {
        self.0.send(Message::text(value.to_string())).await.unwrap();
    }

    /// 读下一帧并解析。对端断开（含核心按协议错误断开）时 panic——脚本假定连接活着。
    pub(super) async fn recv(&mut self) -> Value {
        let message = self
            .0
            .next()
            .await
            .unwrap_or_else(|| panic!("连接断了"))
            .unwrap();
        serde_json::from_str(message.as_text().unwrap()).unwrap()
    }

    /// 读下一帧或连接结束：超长帧一类断开用。
    pub(super) async fn try_recv(&mut self) -> Option<Value> {
        let message = self.0.next().await?;
        Some(
            serde_json::from_str(message.unwrap().as_text().unwrap_or_default())
                .unwrap_or(Value::Null),
        )
    }

    pub(super) async fn close(&mut self) {
        let _ = self.0.send(Message::close(None, "")).await;
        let _ = self.0.next().await;
    }
}

/// 脚本：开会话、发一轮、收 update、取消；返回使用方看到的每一帧。
/// 断线重连后 `session/load` 单独跑（[script_reconnect]）。
pub(super) async fn script(ws: &mut Ws) -> Vec<Value> {
    let mut seen = Vec::new();
    ws.send(json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {"protocolVersion": 1, "clientInfo": {"name": "contract", "version": "0"}},
    }))
    .await;
    seen.push(ws.recv().await);
    ws.send(json!({
        "jsonrpc": "2.0", "id": 2, "method": "session/new",
        "params": {"cwd": "/workspace", "mcpServers": []},
    }))
    .await;
    seen.push(ws.recv().await);
    seen.extend(prompt_round(ws, 3).await);
    ws.send(json!({"jsonrpc": "2.0", "method": "session/cancel", "params": {"sessionId": "s-1"}}))
        .await;
    seen
}

/// 一轮 prompt：先收 update，再收回执。返回这两帧。
pub(super) async fn prompt_round(ws: &mut Ws, id: u64) -> Vec<Value> {
    ws.send(json!({
        "jsonrpc": "2.0", "id": id, "method": "session/prompt",
        "params": {"sessionId": "s-1", "prompt": [{"type": "text", "text": "hello"}]},
    }))
    .await;
    let update = ws.recv().await;
    assert_eq!(update["method"], "session/update");
    let reply = ws.recv().await;
    assert_eq!(reply["id"], id);
    vec![update, reply]
}
