use std::{
    convert::Infallible,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use axum::{
    Router,
    body::{Body, Bytes, to_bytes},
    extract::{Request, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use futures_util::stream;
use serde_json::{Value, json};
use tokio::sync::{Notify, mpsc};

use crate::forward_support::{Server, client, connector, headers, spawn};

type Sender = mpsc::Sender<Result<Bytes, Infallible>>;

#[derive(Clone, Default)]
pub struct Remote {
    pub sender: Arc<Mutex<Option<Sender>>>,
    pub initialize_seen: Arc<Notify>,
    pub accept_gate: Option<Arc<Notify>>,
    pub reply_gate: Option<Arc<Notify>>,
    pub initialize_reply: Option<Value>,
    pub multiline: bool,
    pub connections: Arc<AtomicUsize>,
}

impl Remote {
    pub fn sender(&self) -> Sender {
        self.sender.lock().unwrap().as_ref().unwrap().clone()
    }

    fn frame(&self, value: Value) -> Bytes {
        let json = if self.multiline {
            serde_json::to_string_pretty(&value).unwrap()
        } else {
            value.to_string()
        };
        let mut event = String::from("event: message\nid: remote-event\n");
        for line in json.split('\n') {
            event.push_str("data: ");
            event.push_str(line);
            event.push('\n');
        }
        event.push('\n');
        event.into()
    }

    async fn send(&self, sender: &Sender, value: Value) {
        let _ = sender.send(Ok(self.frame(value))).await;
    }
}

pub fn router(state: Remote) -> Router {
    Router::new()
        .route(
            "/sse",
            get(events).post(|| async { StatusCode::METHOD_NOT_ALLOWED }),
        )
        .route("/messages", post(message))
        .with_state(state)
}

pub async fn start(state: Remote) -> (Server, Server, HeaderMap) {
    let remote = spawn(router(state)).await;
    let connector = connector().await;
    let headers = headers(
        &format!("{}/sse", remote.url()),
        json!({"Authorization":"Bearer test-token"}),
    );
    (remote, connector, headers)
}

async fn events(State(state): State<Remote>) -> Response {
    state.connections.fetch_add(1, Ordering::SeqCst);
    let (sender, receiver) = mpsc::channel(16);
    sender
        .send(Ok(Bytes::from_static(
            b"event: endpoint\ndata: /messages\n\n",
        )))
        .await
        .unwrap();
    *state.sender.lock().unwrap() = Some(sender);
    let stream = stream::unfold(receiver, |mut receiver| async {
        receiver.recv().await.map(|value| (value, receiver))
    });
    (
        [("content-type", "text/event-stream")],
        Body::from_stream(stream),
    )
        .into_response()
}

async fn message(State(state): State<Remote>, request: Request) -> StatusCode {
    assert_eq!(request.headers()["authorization"], "Bearer test-token");
    assert!(
        !request
            .headers()
            .keys()
            .any(|name| name.as_str().starts_with("x-wonton-"))
    );
    let body = to_bytes(request.into_body(), usize::MAX).await.unwrap();
    let message: Value = serde_json::from_slice(&body).unwrap();
    let Some(sender) = state.sender.lock().unwrap().clone() else {
        return StatusCode::NOT_FOUND;
    };
    match message["method"].as_str() {
        Some("initialize") => {
            state.initialize_seen.notify_one();
            if let Some(gate) = &state.accept_gate { gate.notified().await; }
            let reply = state.initialize_reply.clone().unwrap_or_else(|| json!({"jsonrpc":"2.0","id":message["id"],"result":{"protocolVersion":"2024-11-05","capabilities":{"tools":{}},"serverInfo":{"name":"controlled-legacy","version":"1"}}}));
            if let Some(gate) = state.reply_gate.clone() {
                tokio::spawn(async move {
                    tokio::select! {
                        _ = gate.notified() => state.send(&sender, reply).await,
                        _ = sender.closed() => {},
                    }
                });
            } else { state.send(&sender, reply).await; }
        }
        Some("notifications/initialized") => state.send(&sender, json!({"jsonrpc":"2.0","method":"notifications/tools/list_changed"})).await,
        Some("tools/list") => state.send(&sender, json!({"jsonrpc":"2.0","id":message["id"],"result":{"tools":[{"name":"echo","inputSchema":{"type":"object"}}]}})).await,
        Some("tools/call") => {
            state.send(&sender, json!({"jsonrpc":"2.0","method":"notifications/progress","params":{"progressToken":"token","progress":1}})).await;
            state.send(&sender, json!({"jsonrpc":"2.0","id":message["id"],"result":{"content":[{"type":"text","text":"remote-result"}]}})).await;
        }
        Some("wait") => {},
        method => panic!("unexpected test method: {method:?}"),
    }
    StatusCode::ACCEPTED
}

pub async fn initialize(url: String, headers: HeaderMap) -> reqwest::Response {
    client().post(format!("{url}/mcp")).headers(headers)
        .json(&json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test","version":"1"}}})).send().await.unwrap()
}
