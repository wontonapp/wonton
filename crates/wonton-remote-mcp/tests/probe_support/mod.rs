use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{
    Json, Router,
    body::{Body, Bytes},
    extract::State,
    http::{HeaderMap, Method, StatusCode, Uri},
    response::{IntoResponse, Response},
};
use futures_util::StreamExt;
use serde_json::{Value, json};
use tokio::{net::TcpListener, sync::oneshot, task::JoinHandle};
use wonton_remote_mcp::{AddressPolicy, Config};

#[derive(Clone)]
pub struct Options {
    pub status: StatusCode,
    pub sse: bool,
    pub initialize: Value,
    pub raw_initialize: Option<&'static str>,
    pub session: bool,
    pub challenge: Option<String>,
    pub well_known: Option<String>,
    pub metadata: Option<Value>,
    pub html: bool,
    pub get_body: String,
    pub get_status: StatusCode,
    pub notification_status: StatusCode,
    pub tools_status: StatusCode,
    pub tools: Value,
    pub delete_status: StatusCode,
    pub delay_method: Option<&'static str>,
    pub delay_all: Duration,
    pub hanging_body: bool,
    pub hanging_tools_body: bool,
    pub empty_events: usize,
    pub server_request: Option<Value>,
    pub server_reply_status: StatusCode,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            status: StatusCode::OK,
            sse: false,
            initialize: initialize(),
            raw_initialize: None,
            session: true,
            challenge: None,
            well_known: None,
            metadata: None,
            html: false,
            get_body: "event: message\ndata: {}\n\n".to_owned(),
            get_status: StatusCode::OK,
            notification_status: StatusCode::ACCEPTED,
            tools_status: StatusCode::OK,
            tools: json!({"jsonrpc":"2.0","id":2,"result":{"tools":[
                {"name":"search","inputSchema":{"type":"object"}}
            ]}}),
            delete_status: StatusCode::NO_CONTENT,
            delay_method: None,
            delay_all: Duration::ZERO,
            hanging_body: false,
            hanging_tools_body: false,
            empty_events: 0,
            server_request: None,
            server_reply_status: StatusCode::ACCEPTED,
        }
    }
}

pub fn initialize() -> Value {
    json!({"jsonrpc":"2.0","id":1,"result":{
        "protocolVersion":"2025-06-18", "capabilities":{"tools":{}},
        "serverInfo":{"name":"fake-remote","version":"1"}
    }})
}

#[derive(Clone, Debug)]
pub struct Received {
    pub method: Method,
    pub uri: Uri,
    pub headers: HeaderMap,
    pub body: Value,
}

#[derive(Clone)]
struct RemoteState {
    options: Options,
    received: Arc<Mutex<Vec<Received>>>,
    url: String,
    reply_gate: Arc<Mutex<Option<ReplyGate>>>,
}

struct ReplyGate {
    expected: Value,
    completed: oneshot::Sender<()>,
}

pub struct Server {
    pub url: String,
    pub received: Arc<Mutex<Vec<Received>>>,
    task: JoinHandle<()>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Server {
    pub async fn remote(options: Options) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/mcp", listener.local_addr().unwrap());
        let received = Arc::new(Mutex::new(Vec::new()));
        let state = RemoteState {
            options,
            received: received.clone(),
            url: url.clone(),
            reply_gate: Arc::new(Mutex::new(None)),
        };
        let router = Router::new().fallback(remote).with_state(state);
        let task = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        Self {
            url,
            received,
            task,
        }
    }

    pub async fn connector(config: Config) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/_wonton/probe", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            axum::serve(listener, wonton_remote_mcp::router(config))
                .await
                .unwrap();
        });
        Self {
            url,
            received: Arc::new(Mutex::new(Vec::new())),
            task,
        }
    }

    pub fn requests(&self) -> Vec<Received> {
        self.received.lock().unwrap().clone()
    }
}

pub fn config() -> Config {
    Config {
        address_policy: AddressPolicy::AllowPrivate,
        ..Config::default()
    }
}

pub async fn probe(url: &str, config: Config, auth: &str, headers: Value) -> Value {
    let connector = Server::connector(config).await;
    let response = client()
        .post(&connector.url)
        .json(&json!({"url":url,"auth":auth,"headers":headers}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    response.json().await.unwrap()
}

pub async fn simple(remote: &Server) -> Value {
    probe(&remote.url, config(), "none", json!({})).await
}

pub fn client() -> reqwest::Client {
    reqwest::Client::builder().no_proxy().build().unwrap()
}

async fn remote(
    State(state): State<RemoteState>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let body = serde_json::from_slice::<Value>(&body).unwrap_or(Value::Null);
    let rpc_method = body
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    state.received.lock().unwrap().push(Received {
        method: method.clone(),
        uri: uri.clone(),
        headers,
        body: body.clone(),
    });
    let options = &state.options;
    if options.delay_method == Some(rpc_method.as_str())
        || (options.delay_method == Some("DELETE") && method == Method::DELETE)
        || (options.delay_method == Some("server-response")
            && body.get("id").is_some()
            && body.get("method").is_none())
    {
        tokio::time::sleep(Duration::from_secs(60)).await;
    }
    tokio::time::sleep(options.delay_all).await;
    if uri.path().starts_with("/.well-known/") {
        return metadata(&state, uri.path());
    }
    if method == Method::DELETE {
        return options.delete_status.into_response();
    }
    if method == Method::GET {
        return (
            options.get_status,
            [("content-type", "text/event-stream")],
            options.get_body.clone(),
        )
            .into_response();
    }
    if body.get("id").is_some() && body.get("method").is_none() {
        return server_reply(&state, &body);
    }
    match rpc_method.as_str() {
        "notifications/initialized" => options.notification_status.into_response(),
        "tools/list" if options.hanging_tools_body => hanging_response(options.sse),
        "tools/list" => rpc(options.tools.clone(), options.tools_status, &state),
        "initialize" => initialize_response(&state),
        _ => StatusCode::BAD_REQUEST.into_response(),
    }
}

fn initialize_response(state: &RemoteState) -> Response {
    let options = &state.options;
    let mut response = if options.hanging_body {
        hanging_response(options.sse)
    } else if options.html {
        (
            options.status,
            [("content-type", "text/html")],
            "<html>private-body-secret</html>",
        )
            .into_response()
    } else if let Some(raw) = options.raw_initialize {
        let content_type = if options.sse {
            "text/event-stream"
        } else {
            "application/json"
        };
        (options.status, [("content-type", content_type)], raw).into_response()
    } else {
        rpc(options.initialize.clone(), options.status, state)
    };
    if let Some(challenge) = &options.challenge {
        response
            .headers_mut()
            .insert("www-authenticate", challenge.parse().unwrap());
    }
    if options.session && options.status.is_success() {
        response
            .headers_mut()
            .insert("mcp-session-id", "probe-session".parse().unwrap());
    }
    response
}

fn metadata(state: &RemoteState, path: &str) -> Response {
    if state.options.well_known.as_deref() != Some(path) {
        return StatusCode::NOT_FOUND.into_response();
    }
    Json(state.options.metadata.clone().unwrap_or_else(|| {
        json!({
            "resource":state.url,"authorization_servers":["https://auth.example.com"]
        })
    }))
    .into_response()
}

fn rpc(value: Value, status: StatusCode, state: &RemoteState) -> Response {
    if state.options.sse {
        // Exercise comments, a notification, CRLF, and multiline data fields.
        let pretty = serde_json::to_string_pretty(&value).unwrap();
        let data = pretty
            .lines()
            .map(|line| format!("data: {line}\r\n"))
            .collect::<String>();
        let control = "id: cursor-start\r\ndata:\r\n\r\n".repeat(state.options.empty_events);
        let body = format!(
            "{control}: keepalive\r\n\r\ndata: {{\"jsonrpc\":\"2.0\",\"method\":\"notifications/tools/list_changed\"}}\r\n\r\nevent: message\r\n{data}\r\n"
        );
        if let Some(request) = &state.options.server_request {
            return gated_response(state, request, body, status);
        }
        (
            status,
            [("content-type", "text/event-stream; charset=utf-8")],
            body,
        )
            .into_response()
    } else {
        (status, Json(value)).into_response()
    }
}

fn gated_response(
    state: &RemoteState,
    request: &Value,
    body: String,
    status: StatusCode,
) -> Response {
    let expected = if request["method"] == "ping" {
        json!({"jsonrpc":"2.0","id":request["id"],"result":{}})
    } else {
        json!({"jsonrpc":"2.0","id":request["id"],
            "error":{"code":-32601,"message":"Method not found"}})
    };
    let (completed, released) = oneshot::channel();
    let previous = state.reply_gate.lock().unwrap().replace(ReplyGate {
        expected,
        completed,
    });
    assert!(
        previous.is_none(),
        "Previous RPC result must be released first"
    );
    let first = Bytes::from(format!("data: {request}\n\n"));
    let stream = futures_util::stream::once(async { Ok::<_, std::io::Error>(first) }).chain(
        futures_util::stream::once(async move {
            released
                .await
                .map_err(|_| std::io::Error::other("Server request was not answered"))?;
            Ok(Bytes::from(body))
        }),
    );
    Response::builder()
        .status(status)
        .header("content-type", "text/event-stream")
        .body(Body::from_stream(stream))
        .unwrap()
}

fn server_reply(state: &RemoteState, body: &Value) -> Response {
    if !state.options.server_reply_status.is_success() {
        return (state.options.server_reply_status, "private-reply-secret").into_response();
    }
    let mut gate = state.reply_gate.lock().unwrap();
    if !gate.as_ref().is_some_and(|gate| gate.expected == *body) {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let _ = gate.take().unwrap().completed.send(());
    state.options.server_reply_status.into_response()
}

fn hanging_response(sse: bool) -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header(
            "content-type",
            if sse {
                "text/event-stream"
            } else {
                "application/json"
            },
        )
        .body(Body::from_stream(futures_util::stream::pending::<
            Result<Bytes, std::io::Error>,
        >()))
        .unwrap()
}
