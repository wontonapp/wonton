use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};
use axum::{Router, body::{Body, to_bytes}, extract::{Request, State}, response::Response};
use matrix_sdk::reqwest;
use tokio::{net::TcpListener, task::JoinHandle};

pub struct Relay {
    pub url: String,
    state: Arc<RelayState>,
    task: JoinHandle<()>,
}
struct RelayState {
    upstream: String,
    http: reqwest::Client,
    phase: AtomicUsize,
    sequence: AtomicUsize,
}
impl Relay {
    pub async fn start(upstream: &str) -> Self {
        let state = Arc::new(RelayState {
            upstream: upstream.to_owned(), http: reqwest::Client::new(),
            phase: AtomicUsize::new(0), sequence: AtomicUsize::new(0),
        });
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let app = Router::new().fallback(forward).with_state(state.clone());
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); });
        Self { url, state, task }
    }
    pub fn phase(&self, phase: usize, name: &str) {
        self.state.phase.store(phase, Ordering::SeqCst);
        eprintln!("TRACE804 PHASE {phase} {name}");
    }
}
impl Drop for Relay { fn drop(&mut self) { self.task.abort(); } }

async fn forward(State(state): State<Arc<RelayState>>, request: Request) -> Response {
    let (parts, body) = request.into_parts();
    let path = parts.uri.path().to_owned();
    let phase = state.phase.load(Ordering::SeqCst);
    let seq = state.sequence.fetch_add(1, Ordering::SeqCst);
    let method = parts.method.clone();
    eprintln!("TRACE804 HTTP begin seq={seq} phase={phase} {method} {path}");
    let body = to_bytes(body, usize::MAX).await.unwrap();
    let upstream = state.http.request(parts.method, format!("{}{}", state.upstream, parts.uri))
        .headers(parts.headers).body(body).send().await.unwrap();
    let status = upstream.status();
    let headers = upstream.headers().clone();
    let body = upstream.bytes().await.unwrap();
    eprintln!("TRACE804 HTTP end seq={seq} phase={phase} {method} {path} status={status}");
    if path.ends_with("/sync") || path.ends_with("/messages") || path.contains("/room_keys/keys") {
        if let Ok(json) = serde_json::from_slice::<serde_json::Value>(&body) {
            if let Some(rooms) = json.get("rooms").and_then(|v| v.as_object()) {
                for (room_id, room) in rooms {
                    if let Some(events) = room.get("timeline").and_then(|v| v.as_array()) {
                        for event in events { record_event(seq, phase, room_id, event); }
                    }
                }
            }
            if let Some(events) = json.get("chunk").and_then(|v| v.as_array()) {
                for event in events { record_event(seq, phase, &path, event); }
            }
        }
    }
    let mut response = Response::new(Body::from(body));
    *response.status_mut() = status;
    *response.headers_mut() = headers;
    response.headers_mut().remove("transfer-encoding");
    response.headers_mut().remove("content-length");
    response
}
fn record_event(seq: usize, phase: usize, room: &str, event: &serde_json::Value) {
    if event["type"] == "m.room.encrypted" {
        eprintln!("TRACE804 EVENT seq={seq} phase={phase} room={room} event_id={} session_id={}", event["event_id"], event["content"]["session_id"]);
    }
}
