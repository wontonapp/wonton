//! Reusable local ACP contract provider. Default mode completes each prompt immediately;
//! controlled mode keeps prompts pending while continuing to read cancellation frames.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Weak};
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
use tokio::sync::{mpsc, oneshot};
use tokio::task::AbortHandle;
use tokio_websockets::{Message, ServerBuilder, WebSocketStream};

static NEXT_SESSION: AtomicUsize = AtomicUsize::new(1);
type Ws = WebSocketStream<TokioIo<hyper::upgrade::Upgraded>>;
type Commands = mpsc::UnboundedSender<Command>;

#[derive(Default)]
pub(crate) struct Recording {
    pub(crate) frames: Mutex<Vec<(String, String)>>,
    pub(crate) users: Mutex<Vec<String>>,
    pub(crate) headers: Mutex<Vec<HashMap<String, String>>>,
    connections: AtomicUsize,
    wire_frames: AtomicUsize,
    sessions: Mutex<HashMap<String, Commands>>,
    tasks: Mutex<Vec<AbortHandle>>,
}

impl Drop for Recording {
    fn drop(&mut self) {
        for task in self.tasks.get_mut().unwrap().drain(..) {
            task.abort();
        }
    }
}

impl Recording {
    pub(crate) fn method_count(&self, method: &str) -> usize {
        self.frames
            .lock()
            .unwrap()
            .iter()
            .filter(|(m, _)| m == method)
            .count()
    }

    pub(crate) fn connections(&self) -> usize {
        self.connections.load(Ordering::SeqCst)
    }

    /// All inbound WebSocket messages, including non-text and malformed JSON.
    pub(crate) fn frame_count(&self) -> usize {
        self.wire_frames.load(Ordering::SeqCst)
    }

    pub(crate) fn received(&self, method: &str) -> Vec<Value> {
        self.frames
            .lock()
            .unwrap()
            .iter()
            .filter(|(seen, _)| seen == method)
            .map(|(_, text)| serde_json::from_str(text).unwrap())
            .collect()
    }

    pub(crate) async fn last_user(&self, user: &str) {
        for _ in 0..40 {
            if self
                .users
                .lock()
                .unwrap()
                .last()
                .is_some_and(|seen| seen == user)
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("提供方没收到说话人 {user}");
    }

    /// Wait for the zero-based prompt ordinal, without consuming the recording.
    pub(crate) async fn wait_prompt(&self, ordinal: usize) -> Value {
        for _ in 0..40 {
            if let Some(frame) = self.received("session/prompt").get(ordinal) {
                return frame.clone();
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("provider did not receive prompt {ordinal}");
    }
}

pub(crate) struct ControlledProvider {
    pub(crate) addr: std::net::SocketAddr,
    pub(crate) recording: Arc<Recording>,
}

impl Drop for ControlledProvider {
    fn drop(&mut self) {
        for task in self.recording.tasks.lock().unwrap().drain(..) {
            task.abort();
        }
    }
}

impl ControlledProvider {
    pub(crate) async fn text(&self, session: &str, text: impl Into<String>) {
        self.command(session, Some(text.into())).await;
    }

    pub(crate) async fn finish(&self, session: &str) {
        self.command(session, None).await;
    }

    async fn command(&self, session: &str, text: Option<String>) {
        let sender = self
            .recording
            .sessions
            .lock()
            .unwrap()
            .get(session)
            .unwrap_or_else(|| panic!("unknown provider session {session}"))
            .clone();
        let (done, received) = oneshot::channel();
        sender
            .send(Command {
                session: session.to_owned(),
                text,
                done,
            })
            .unwrap();
        tokio::time::timeout(Duration::from_secs(2), received)
            .await
            .expect("provider command timed out")
            .expect("provider disconnected");
    }
}

#[derive(Debug)]
struct Command {
    session: String,
    text: Option<String>,
    done: oneshot::Sender<()>,
}

#[derive(Clone)]
struct ProviderState {
    token: String,
    recording: Weak<Recording>,
    controlled: bool,
}

/// The returned recording owns and aborts the server and all connection tasks.
pub(crate) async fn serve(token: String) -> (std::net::SocketAddr, Arc<Recording>) {
    start(token, false).await
}

pub(crate) async fn serve_controlled(token: String) -> ControlledProvider {
    let (addr, recording) = start(token, true).await;
    ControlledProvider { addr, recording }
}

async fn start(token: String, controlled: bool) -> (std::net::SocketAddr, Arc<Recording>) {
    let recording = Arc::new(Recording::default());
    let state = ProviderState {
        token,
        recording: Arc::downgrade(&recording),
        controlled,
    };
    let app = axum::Router::new()
        .route("/acp", axum::routing::get(entry))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    recording.tasks.lock().unwrap().push(task.abort_handle());
    (addr, recording)
}

async fn entry(State(state): State<ProviderState>, mut req: Request<Body>) -> Response {
    let Some(recording) = state.recording.upgrade() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    recording.connections.fetch_add(1, Ordering::SeqCst);
    let headers: HashMap<String, String> = req
        .headers()
        .iter()
        .filter(|(name, _)| name.as_str().starts_with("x-wonton-"))
        .filter_map(|(name, value)| Some((name.to_string(), value.to_str().ok()?.to_owned())))
        .collect();
    if let Some(user) = headers.get("x-wonton-user") {
        recording.users.lock().unwrap().push(user.clone());
    }
    recording.headers.lock().unwrap().push(headers);
    let authorized = req
        .headers()
        .get(hyper::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value == format!("Bearer {}", state.token));
    if !authorized {
        return (
            StatusCode::UNAUTHORIZED,
            "agent_acp provider requires its bearer token",
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
    let task = tokio::spawn(serve_frames(on_upgrade, state));
    recording.tasks.lock().unwrap().push(task.abort_handle());
    Response::builder()
        .status(StatusCode::SWITCHING_PROTOCOLS)
        .header(hyper::header::CONNECTION, "Upgrade")
        .header(hyper::header::UPGRADE, "websocket")
        .header(hyper::header::SEC_WEBSOCKET_ACCEPT, accept)
        .body(Body::empty())
        .unwrap()
}

async fn serve_frames(on_upgrade: hyper::upgrade::OnUpgrade, state: ProviderState) {
    let Ok(upgraded) = on_upgrade.await else {
        return;
    };
    let mut ws = ServerBuilder::new().serve(TokioIo::new(upgraded));
    let (sender, mut commands) = mpsc::unbounded_channel::<Command>();
    let mut pending = HashMap::<String, Value>::new();
    let mut sessions = 0u64;
    loop {
        tokio::select! {
            message = ws.next() => {
                let Some(Ok(message)) = message else { break };
                let Some(recording) = state.recording.upgrade() else { break };
                recording.wire_frames.fetch_add(1, Ordering::SeqCst);
                if !message.is_text() { continue; }
                let text = message.as_text().unwrap();
                let Ok(frame) = serde_json::from_str::<Value>(text) else { continue };
                let method = frame["method"].as_str().unwrap_or_default().to_owned();
                recording.frames.lock().unwrap().push((method.clone(), text.to_owned()));
                drop(recording);
                if !handle_frame(&mut ws, &state, &sender, &mut pending, &method, frame, &mut sessions).await {
                    break;
                }
            }
            Some(command) = commands.recv() => {
                let Some(id) = pending.get(&command.session) else {
                    panic!("command requires pending prompt for {}", command.session);
                };
                let sent = if let Some(text) = command.text {
                    update(&mut ws, &command.session, &text).await
                } else {
                    let id = id.clone();
                    pending.remove(&command.session);
                    reply(&mut ws, id, json!({"stopReason": "end_turn"})).await
                };
                if !sent { break; }
                let _ = command.done.send(());
            }
        }
    }
}

async fn handle_frame(
    ws: &mut Ws,
    state: &ProviderState,
    sender: &Commands,
    pending: &mut HashMap<String, Value>,
    method: &str,
    frame: Value,
    sessions: &mut u64,
) -> bool {
    let session = frame["params"]["sessionId"].as_str().unwrap_or("s-1");
    match (method, frame.get("id").cloned()) {
        ("initialize", Some(id)) => {
            reply(
                ws,
                id,
                json!({
                    "protocolVersion": 1, "agentCapabilities": {"loadSession": true},
                    "authMethods": [], "agentInfo": {"name": "contract-provider", "version": "0"},
                }),
            )
            .await
        }
        ("session/new", Some(id)) => {
            *sessions += 1;
            let session = if state.controlled {
                format!("controlled-{}", NEXT_SESSION.fetch_add(1, Ordering::SeqCst))
            } else {
                format!("s-{sessions}")
            };
            if let Some(recording) = state.recording.upgrade() {
                recording
                    .sessions
                    .lock()
                    .unwrap()
                    .insert(session.clone(), sender.clone());
            }
            reply(ws, id, json!({"sessionId": session})).await
        }
        ("session/load", Some(id)) => reply(ws, id, json!({"sessionId": session})).await,
        ("session/prompt", Some(id)) if state.controlled => {
            assert!(
                pending.insert(session.to_owned(), id).is_none(),
                "overlapping session prompt"
            );
            true
        }
        ("session/prompt", Some(id)) => {
            update(ws, session, "hello").await
                && reply(ws, id, json!({"stopReason": "end_turn"})).await
        }
        ("session/cancel", _) => {
            if let Some(id) = pending.remove(session) {
                return reply(ws, id, json!({"stopReason": "cancelled"})).await;
            }
            true
        }
        _ => true,
    }
}

async fn update(ws: &mut Ws, session: &str, text: &str) -> bool {
    ws.send(Message::text(
        json!({
            "jsonrpc": "2.0", "method": "session/update", "params": {
                "sessionId": session, "update": {"sessionUpdate": "agent_message_chunk",
                    "content": {"type": "text", "text": text}},
            },
        })
        .to_string(),
    ))
    .await
    .is_ok()
}

async fn reply(ws: &mut Ws, id: Value, result: Value) -> bool {
    ws.send(Message::text(
        json!({"jsonrpc": "2.0", "id": id, "result": result}).to_string(),
    ))
    .await
    .is_ok()
}
