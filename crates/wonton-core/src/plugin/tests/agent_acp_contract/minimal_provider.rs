//! 只照契约写的最小提供方（ADR-0061 第 1 节的契约测试一侧）：`/acp` 上的 WebSocket，认提供方令牌，
//! 回 `initialize`、`session/new`、`session/prompt`（先推一条 `session/update` 再回结束）、`session/load`，
//! 收下一切并记录。同一段使用方脚本对它和经核心转发各跑一遍，使用方看到的报文应当一样。

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use aws_lc_rs::digest;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use futures_util::{SinkExt, StreamExt};
use hyper_util::rt::TokioIo;
use serde_json::{Value, json};
use tokio_websockets::{Message, ServerBuilder, WebSocketStream};

/// 一条连接上的收发记录：收到的帧（方法，原文）、握手头里的说话人、收到的连接数。给断言用。
#[derive(Default)]
pub(super) struct Recording {
    pub(super) frames: Mutex<Vec<(String, String)>>,
    pub(super) users: Mutex<Vec<String>>,
    pub(super) connections: AtomicUsize,
}

impl Recording {
    pub(super) fn method_count(&self, method: &str) -> usize {
        self.frames
            .lock()
            .expect("提供方记录锁中毒")
            .iter()
            .filter(|(m, _)| m == method)
            .count()
    }

    /// 核心算出的说话人：提供方握手头里的最后一个 X-Wonton-User。
    pub(super) async fn last_user(&self, user: &str) {
        for _ in 0..40 {
            if self
                .users
                .lock()
                .expect("提供方记录锁中毒")
                .last()
                .is_some_and(|seen| seen == user)
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("提供方没收到说话人 {user}");
    }

    pub(super) fn connections(&self) -> usize {
        self.connections.load(Ordering::SeqCst)
    }
}

/// 起一个最小提供方，返回（地址，记录）。令牌由核心注入的那把来。
pub(super) async fn serve(token: String) -> (std::net::SocketAddr, Arc<Recording>) {
    let recording = Arc::new(Recording::default());
    let app = axum::Router::new()
        .route("/acp", axum::routing::get(entry))
        .with_state((token, recording.clone()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (addr, recording)
}

/// 认令牌（Authorization: Bearer）后升级；没有令牌的连接（同插件网络里伪造的）在这里被拒。
async fn entry(
    State((token, recording)): State<(String, Arc<Recording>)>,
    mut req: Request<Body>,
) -> Response {
    recording.connections.fetch_add(1, Ordering::SeqCst);
    if let Some(user) = req
        .headers()
        .get("x-wonton-user")
        .and_then(|value| value.to_str().ok())
    {
        recording
            .users
            .lock()
            .expect("提供方记录锁中毒")
            .push(user.to_owned());
    }
    let authorized = req
        .headers()
        .get(hyper::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value == format!("Bearer {token}"));
    if !authorized {
        return (
            StatusCode::UNAUTHORIZED,
            "agent_acp 提供方只认核心转发来、带提供方令牌的连接",
        )
            .into_response();
    }
    let Some(key) = req
        .headers()
        .get(hyper::header::SEC_WEBSOCKET_KEY)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
    else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let mut hash = digest::Context::new(&digest::SHA1_FOR_LEGACY_USE_ONLY);
    hash.update(key.as_bytes());
    hash.update(b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11");
    let accept = STANDARD.encode(hash.finish().as_ref());
    let on_upgrade = hyper::upgrade::on(&mut req);
    tokio::spawn(serve_frames(on_upgrade, recording));
    Response::builder()
        .status(StatusCode::SWITCHING_PROTOCOLS)
        .header(hyper::header::CONNECTION, "Upgrade")
        .header(hyper::header::UPGRADE, "websocket")
        .header(hyper::header::SEC_WEBSOCKET_ACCEPT, accept)
        .body(Body::empty())
        .unwrap()
}

async fn serve_frames(on_upgrade: hyper::upgrade::OnUpgrade, recording: Arc<Recording>) {
    let upgraded = match on_upgrade.await {
        Ok(upgraded) => TokioIo::new(upgraded),
        Err(_) => return,
    };
    let mut ws: WebSocketStream<TokioIo<hyper::upgrade::Upgraded>> =
        ServerBuilder::new().serve(upgraded);
    while let Some(Ok(message)) = ws.next().await {
        if !message.is_text() {
            continue;
        }
        let frame: Value = match serde_json::from_str(message.as_text().unwrap_or_default()) {
            Ok(frame) => frame,
            Err(_) => continue,
        };
        let id = frame.get("id").cloned();
        let method = frame
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        recording
            .frames
            .lock()
            .expect("提供方记录锁中毒")
            .push((method.clone(), message.as_text().unwrap().to_owned()));
        match (method.as_str(), id) {
            ("initialize", Some(id)) => {
                reply(&mut ws, id, json!({"protocolVersion": 1})).await;
            }
            ("session/new" | "session/load", Some(id)) => {
                reply(&mut ws, id, json!({"sessionId": "s-1"})).await;
            }
            ("session/prompt", Some(id)) => {
                ws.send(Message::text(
                    json!({
                        "jsonrpc": "2.0",
                        "method": "session/update",
                        "params": {"sessionId": "s-1", "update": {"sessionUpdate": "agent_message_chunk"}},
                    })
                    .to_string(),
                ))
                .await
                .unwrap();
                reply(&mut ws, id, json!({"stopReason": "end_turn"})).await;
            }
            _ => {}
        }
    }
}

async fn reply(
    ws: &mut WebSocketStream<TokioIo<hyper::upgrade::Upgraded>>,
    id: Value,
    result: Value,
) {
    ws.send(Message::text(
        json!({"jsonrpc": "2.0", "id": id, "result": result}).to_string(),
    ))
    .await
    .unwrap();
}
