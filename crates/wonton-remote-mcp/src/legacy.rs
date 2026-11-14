use std::{
    collections::HashMap,
    io,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, Ordering},
    },
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
use serde_json::Value;
use sse_stream::{Sse, SseStream};
use tokio::sync::{mpsc, oneshot};
use url::Url;

use crate::{
    failure,
    headers::RemoteRequest,
    network::{Network, NetworkError},
    network_failure,
};

type Event = Result<Bytes, io::Error>;
type Pending = HashMap<String, oneshot::Sender<Event>>;

pub(crate) enum OpenError {
    Network(NetworkError),
    Protocol(&'static str),
}

impl IntoResponse for OpenError {
    fn into_response(self) -> Response {
        match self {
            Self::Network(error) => network_failure(error),
            Self::Protocol(detail) => legacy_failure(detail),
        }
    }
}

#[derive(Default)]
pub(crate) struct Sessions(Mutex<HashMap<String, Arc<Session>>>);

impl Sessions {
    pub fn find(&self, id: &str, context: &RemoteRequest) -> Option<Arc<Session>> {
        self.0
            .lock()
            .expect("sessions lock")
            .get(id)
            .filter(|s| s.matches(context))
            .cloned()
    }

    pub fn insert(&self, session: Arc<Session>) {
        self.0
            .lock()
            .expect("sessions lock")
            .insert(session.id.clone(), session);
    }

    pub fn remove(&self, id: &str) {
        if let Some(session) = self.0.lock().expect("sessions lock").remove(id) {
            session.close();
        }
    }

    pub fn forget(&self, plugin: &str) {
        self.0.lock().expect("sessions lock").retain(|_, session| {
            if session.plugin != plugin {
                return true;
            }
            session.close();
            false
        });
    }
}

pub(crate) struct Session {
    pub id: String,
    plugin: String,
    url: Url,
    credentials: HeaderMap,
    endpoint: Url,
    pending: Mutex<Pending>,
    events: Mutex<Option<mpsc::Receiver<Event>>>,
    reader: Mutex<Option<tokio::task::AbortHandle>>,
    alive: AtomicBool,
}

impl Drop for Session {
    fn drop(&mut self) {
        if let Some(task) = self.reader.get_mut().expect("reader lock").take() {
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

    fn close(&self) {
        self.alive.store(false, Ordering::Release);
        self.pending.lock().expect("pending lock").clear();
        if let Some(task) = self.reader.lock().expect("reader lock").take() {
            task.abort();
        }
    }

    pub async fn open(
        network: &Network,
        context: &RemoteRequest,
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
        let mut events = SseStream::from_bytes_stream(response.bytes_stream()).boxed();
        let first = tokio::time::timeout(network.config().connect_timeout, async {
            while let Some(event) = events.next().await {
                let event =
                    event.map_err(|_| OpenError::Protocol("Invalid legacy event stream"))?;
                if event.event.is_some() || event.data.is_some() {
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
            pending: Mutex::new(HashMap::new()),
            events: Mutex::new(Some(rx)),
            reader: Mutex::new(None),
            alive: AtomicBool::new(true),
        });
        let weak = Arc::downgrade(&session);
        let task = tokio::spawn(read_events(events, weak, tx));
        *session.reader.lock().expect("reader lock") = Some(task.abort_handle());
        Ok(Some(session))
    }

    pub fn events(&self) -> Response {
        let Some(receiver) = self.events.lock().expect("events lock").take() else {
            return failure(
                StatusCode::CONFLICT,
                "remote_error",
                "Legacy event stream already has a consumer",
            );
        };
        let stream = stream::unfold(receiver, |mut rx| async {
            rx.recv().await.map(|event| (event, rx))
        });
        sse_response(Body::from_stream(stream), &self.id)
    }

    pub async fn post(
        self: &Arc<Self>,
        network: &Network,
        headers: HeaderMap,
        body: Bytes,
    ) -> Response {
        if !self.alive.load(Ordering::Acquire) {
            return legacy_failure("Legacy event stream is closed");
        }
        let message: Value = match serde_json::from_slice(&body) {
            Ok(message) => message,
            Err(_) => {
                return failure(
                    StatusCode::BAD_REQUEST,
                    "invalid_input",
                    "Legacy transport requires JSON-RPC messages",
                );
            }
        };
        let id = message
            .get("id")
            .filter(|_| message.get("method").is_some())
            .map(Value::to_string);
        let mut guard = None;
        let receiver = if let Some(id) = id {
            let (sender, receiver) = oneshot::channel();
            let mut pending = self.pending.lock().expect("pending lock");
            if pending.contains_key(&id) {
                return failure(
                    StatusCode::BAD_REQUEST,
                    "invalid_input",
                    "Duplicate active JSON-RPC request ID",
                );
            }
            pending.insert(id.clone(), sender);
            guard = Some(PendingGuard {
                session: self.clone(),
                id,
            });
            Some(receiver)
        } else {
            None
        };
        let mut headers = headers;
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
        let Some(receiver) = receiver else {
            return StatusCode::ACCEPTED.into_response();
        };
        let stream = stream::once(async move {
            let _guard = guard;
            receiver.await.unwrap_or_else(|_| {
                Err(io::Error::other(
                    "Legacy event stream closed before the response",
                ))
            })
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
            .pending
            .lock()
            .expect("pending lock")
            .remove(&self.id);
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
    mut events: BoxStream<'static, Result<Sse, sse_stream::Error>>,
    weak: Weak<Session>,
    tx: mpsc::Sender<Event>,
) {
    while let Some(event) = events.next().await {
        let Some(session) = weak.upgrade() else {
            return;
        };
        let Ok(event) = event else {
            break;
        };
        let Some(data) = event.data.as_deref() else {
            continue;
        };
        let message: Value = match serde_json::from_str(data) {
            Ok(value) => value,
            Err(_) => break,
        };
        let id = message
            .get("id")
            .filter(|_| message.get("method").is_none())
            .map(Value::to_string);
        let bytes: Bytes = event.into();
        if let Some(id) = id {
            if let Some(sender) = session.pending.lock().expect("pending lock").remove(&id) {
                let _ = sender.send(Ok(bytes));
            }
        } else if tx.send(Ok(bytes)).await.is_err() {
            break;
        }
    }
    if let Some(session) = weak.upgrade() {
        session.alive.store(false, Ordering::Release);
        session.pending.lock().expect("pending lock").clear();
    }
    let _ = tx
        .send(Err(io::Error::other("Legacy remote event stream ended")))
        .await;
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

fn legacy_failure(detail: &str) -> Response {
    failure(StatusCode::BAD_GATEWAY, "remote_error", detail)
}
