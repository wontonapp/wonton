mod registry;

use std::{
    collections::HashMap,
    io,
    sync::{Arc, Mutex, Weak},
};

use axum::{
    body::{Body, Bytes},
    http::{HeaderMap, HeaderValue, Method, StatusCode},
    response::{IntoResponse, Response},
};
use base64::Engine;
use futures_util::{
    StreamExt,
    stream::{self, BoxStream},
};
use rmcp::model::InitializeResult;
use serde::Deserialize;
use serde_json::Value;
use sse_stream::Sse;
use tokio::sync::{mpsc, oneshot};
use url::Url;

use crate::{
    failure,
    headers::RemoteRequest,
    network::{Network, NetworkError},
    network_failure,
    sse::{self, EventError},
};

pub(crate) use registry::{Opening, Sessions, cancelled};

type Event = Result<Bytes, io::Error>;
type PendingReply = oneshot::Receiver<Reply>;

struct Reply {
    bytes: Bytes,
    initialized: bool,
}

struct Pending {
    sender: oneshot::Sender<Reply>,
    initialize: bool,
}

struct State {
    closed: bool,
    reason: &'static str,
    pending: HashMap<String, Pending>,
    events: Option<mpsc::Receiver<Event>>,
    reader: Option<tokio::task::AbortHandle>,
}

pub(crate) enum OpenError {
    Network(NetworkError),
    Protocol(&'static str),
    InvalidInput(&'static str),
    Expired,
}

impl IntoResponse for OpenError {
    fn into_response(self) -> Response {
        match self {
            Self::Network(error) => network_failure(error),
            Self::Protocol(detail) => legacy_failure(detail),
            Self::InvalidInput(detail) => failure(StatusCode::BAD_REQUEST, "invalid_input", detail),
            Self::Expired => expired(),
        }
    }
}

pub(crate) struct Session {
    pub id: String,
    plugin: String,
    url: Url,
    credentials: HeaderMap,
    endpoint: Url,
    registry: Weak<Mutex<registry::Registry>>,
    state: Mutex<State>,
}

impl Drop for Session {
    fn drop(&mut self) {
        if let Some(task) = self.state.get_mut().expect("session lock").reader.take() {
            task.abort();
        }
    }
}

impl Session {
    fn matches(&self, context: &RemoteRequest) -> bool {
        self.plugin == context.plugin
            && self.url == context.url
            && credentials(&context.headers) == self.credentials
    }

    fn is_open(&self) -> bool {
        !self.state.lock().expect("session lock").closed
    }

    fn close(&self, reason: &'static str) {
        let reader = {
            let mut state = self.state.lock().expect("session lock");
            if state.closed {
                return;
            }
            state.closed = true;
            state.reason = reason;
            state.pending.clear();
            state.events.take();
            state.reader.take()
        };
        if let Some(reader) = reader {
            reader.abort();
        }
        registry::remove_closed(&self.registry, &self.id);
    }

    pub async fn open(
        network: &Network,
        context: &RemoteRequest,
        opening: &Opening,
    ) -> Result<Option<Arc<Self>>, OpenError> {
        let mut headers = context.headers.clone();
        headers.insert("accept", HeaderValue::from_static("text/event-stream"));
        headers.remove("content-type");
        let response = network
            .send(Method::GET, &context.url, headers, None)
            .await
            .map_err(OpenError::Network)?;
        if !response.status().is_success()
            || !response
                .headers()
                .get("content-type")
                .and_then(|h| h.to_str().ok())
                .is_some_and(|s| s.starts_with("text/event-stream"))
        {
            return Ok(None);
        }
        let mut events = sse::events(response).boxed();
        let first = tokio::time::timeout(network.config().connect_timeout, async {
            while let Some(event) = events.next().await {
                let event = event
                    .map_err(|_| OpenError::Protocol("Invalid or oversized legacy event stream"))?;
                if event.event.is_some()
                    || event.data.as_deref().is_some_and(|data| !data.is_empty())
                {
                    return Ok(Some(event));
                }
            }
            Ok(None)
        })
        .await
        .map_err(|_| OpenError::Protocol("Legacy endpoint discovery timed out"))??;
        let Some(first) = first.filter(|s| s.event.as_deref() == Some("endpoint")) else {
            return Ok(None);
        };
        let endpoint = first
            .data
            .as_deref()
            .and_then(|s| context.url.join(s).ok())
            .ok_or(OpenError::Protocol("Invalid legacy message endpoint"))?;
        if endpoint.origin() != context.url.origin() {
            return Err(OpenError::Protocol(
                "Legacy message endpoint must have the same origin",
            ));
        }
        let mut random = [0; 24];
        getrandom::fill(&mut random)
            .map_err(|_| OpenError::Protocol("Could not generate connector session ID"))?;
        let id = format!(
            "wonton-legacy-{}",
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(random)
        );
        let (tx, rx) = mpsc::channel(64);
        let session = Arc::new(Self {
            id,
            plugin: context.plugin.clone(),
            url: context.url.clone(),
            credentials: credentials(&context.headers),
            endpoint,
            registry: opening.registry(),
            state: Mutex::new(State {
                closed: false,
                reason: "Legacy session closed",
                pending: HashMap::new(),
                events: Some(rx),
                reader: None,
            }),
        });
        if !opening.attach(&session) {
            return Err(OpenError::Expired);
        }
        let task = tokio::spawn(read_events(events, Arc::downgrade(&session), tx));
        {
            let mut state = session.state.lock().expect("session lock");
            if state.closed {
                task.abort();
            } else {
                state.reader = Some(task.abort_handle());
            }
        }
        Ok(Some(session))
    }

    pub fn events(self: &Arc<Self>) -> Response {
        let receiver = {
            let mut state = self.state.lock().expect("session lock");
            if state.closed {
                return expired();
            }
            let Some(receiver) = state.events.take() else {
                return failure(
                    StatusCode::CONFLICT,
                    "remote_error",
                    "Legacy event stream already has a consumer",
                );
            };
            receiver
        };
        let subscription = Subscription {
            session: self.clone(),
            receiver,
            ended: false,
        };
        sse_response(Body::from_stream(subscription), &self.id)
    }

    fn register(
        self: &Arc<Self>,
        body: &[u8],
        initialize: bool,
    ) -> Result<Option<(PendingReply, PendingGuard)>, OpenError> {
        let message: Value = serde_json::from_slice(body)
            .map_err(|_| OpenError::InvalidInput("Legacy transport requires JSON-RPC messages"))?;
        let id = message
            .get("id")
            .filter(|_| message.get("method").is_some())
            .map(Value::to_string);
        let mut state = self.state.lock().expect("session lock");
        if state.closed {
            return Err(OpenError::Expired);
        }
        let Some(id) = id else {
            if initialize {
                return Err(OpenError::InvalidInput(
                    "Legacy initialization requires a request ID",
                ));
            }
            return Ok(None);
        };
        if state.pending.contains_key(&id) {
            return Err(OpenError::InvalidInput(
                "Duplicate active JSON-RPC request ID",
            ));
        }
        let (sender, receiver) = oneshot::channel();
        state
            .pending
            .insert(id.clone(), Pending { sender, initialize });
        Ok(Some((
            receiver,
            PendingGuard {
                session: self.clone(),
                id,
            },
        )))
    }

    pub async fn post(
        self: &Arc<Self>,
        network: &Network,
        mut headers: HeaderMap,
        body: Bytes,
        opening: Option<Opening>,
    ) -> Response {
        let registration = match self.register(&body, opening.is_some()) {
            Ok(registration) => registration,
            Err(error) => return error.into_response(),
        };
        headers.remove("mcp-session-id");
        let response = match network
            .send(Method::POST, &self.endpoint, headers, Some(body.into()))
            .await
        {
            Ok(response) => response,
            Err(error) => return network_failure(error),
        };
        if !response.status().is_success() {
            return crate::forward::response(response);
        }
        let Some((receiver, guard)) = registration else {
            return StatusCode::ACCEPTED.into_response();
        };
        let stream = stream::once(async move {
            let guard = guard;
            let reply = receiver
                .await
                .map_err(|_| io::Error::other("Legacy event stream closed before the response"))?;
            if let Some(opening) = opening {
                if reply.initialized {
                    if !opening.publish() {
                        return Err(io::Error::other("Legacy initialization was cancelled"));
                    }
                } else {
                    guard.session.close("Legacy initialization failed");
                }
            }
            Ok::<_, io::Error>(reply.bytes)
        });
        sse_response(Body::from_stream(stream), &self.id)
    }
}

struct PendingGuard {
    session: Arc<Session>,
    id: String,
}
impl Drop for PendingGuard {
    fn drop(&mut self) {
        self.session
            .state
            .lock()
            .expect("session lock")
            .pending
            .remove(&self.id);
    }
}

struct Subscription {
    session: Arc<Session>,
    receiver: mpsc::Receiver<Event>,
    ended: bool,
}
impl futures_util::Stream for Subscription {
    type Item = Event;
    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Event>> {
        if self.ended {
            return std::task::Poll::Ready(None);
        }
        match self.receiver.poll_recv(cx) {
            std::task::Poll::Ready(None) => {
                self.ended = true;
                let reason = self.session.state.lock().expect("session lock").reason;
                std::task::Poll::Ready(Some(Err(io::Error::other(reason))))
            }
            next => next,
        }
    }
}
impl Drop for Subscription {
    fn drop(&mut self) {
        // Legacy servers cannot replay bytes consumed by the departed HTTP client.
        // Expire the whole session so rmcp reinitializes on 404 instead of retrying 409.
        self.session.close("Legacy event consumer disconnected");
    }
}

fn credentials(headers: &HeaderMap) -> HeaderMap {
    let mut headers = headers.clone();
    for name in [
        "content-type",
        "accept",
        "mcp-session-id",
        "mcp-protocol-version",
        "last-event-id",
    ] {
        headers.remove(name);
    }
    headers
}

async fn read_events(
    mut events: BoxStream<'static, Result<Sse, EventError>>,
    weak: Weak<Session>,
    tx: mpsc::Sender<Event>,
) {
    while let Some(event) = events.next().await {
        let Some(session) = weak.upgrade() else {
            return;
        };
        let event = match event {
            Ok(event) => event,
            Err(error) => {
                tracing::warn!(%error, "Legacy event stream rejected");
                session.close("Invalid or oversized legacy event stream");
                return;
            }
        };
        let Some(data) = event.data.as_deref().filter(|data| !data.is_empty()) else {
            continue;
        };
        let message: Value = match serde_json::from_str(data) {
            Ok(value) => value,
            Err(_) => {
                session.close("Invalid legacy JSON-RPC event");
                return;
            }
        };
        let id = message
            .get("id")
            .filter(|_| message.get("method").is_none())
            .map(Value::to_string);
        if let Some(id) = id {
            let pending = session
                .state
                .lock()
                .expect("session lock")
                .pending
                .remove(&id);
            if let Some(pending) = pending {
                let initialized = pending.initialize && valid_initialize(&message);
                let _ = pending.sender.send(Reply {
                    bytes: sse::encode(event),
                    initialized,
                });
            }
        } else if tx.send(Ok(sse::encode(event))).await.is_err() {
            session.close("Legacy event consumer disconnected");
            return;
        }
    }
    if let Some(session) = weak.upgrade() {
        session.close("Legacy remote event stream ended");
    }
}

fn valid_initialize(message: &Value) -> bool {
    message.get("jsonrpc").and_then(Value::as_str) == Some("2.0")
        && message.get("error").is_none()
        && message
            .get("result")
            .is_some_and(|result| InitializeResult::deserialize(result).is_ok())
}

fn sse_response(body: Body, id: &str) -> Response {
    let mut response = body.into_response();
    response.headers_mut().insert(
        "content-type",
        HeaderValue::from_static("text/event-stream"),
    );
    response
        .headers_mut()
        .insert("cache-control", HeaderValue::from_static("no-cache"));
    response.headers_mut().insert(
        "mcp-session-id",
        HeaderValue::from_str(id).expect("base64 session ID"),
    );
    response
}

pub(crate) fn expired() -> Response {
    failure(
        StatusCode::NOT_FOUND,
        "remote_error",
        "Legacy connector session expired",
    )
}
fn legacy_failure(detail: &str) -> Response {
    failure(StatusCode::BAD_GATEWAY, "remote_error", detail)
}

#[cfg(test)]
#[path = "legacy_tests.rs"]
mod legacy_tests;
