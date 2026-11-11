use std::convert::Infallible;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use super::response_upstream::responses;
use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::StreamExt;
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::sync::Notify;

#[derive(Clone, Default)]
struct Script {
    requests: Arc<Mutex<Vec<Value>>>,
    release: Arc<Notify>,
}

pub(super) struct Upstream {
    pub port: u16,
    script: Script,
    server: tokio::task::JoinHandle<()>,
    pub socket: PathBuf,
    unix_server: tokio::task::JoinHandle<()>,
    _dir: tempfile::TempDir,
}

impl Upstream {
    pub async fn start() -> Self {
        let script = Script::default();
        let app = Router::new()
            .route(
                "/v1/models",
                get(|| async {
                    Json(json!({"object":"list","data":[
                        {"id":"contract-text","object":"model","created":0,"owned_by":"fixture"}
                    ]}))
                }),
            )
            .route("/v1/chat/completions", post(chat))
            .route("/v1/responses", post(responses))
            .route("/down/v1/models", get(unavailable))
            .route("/down/v1/chat/completions", post(unavailable))
            .with_state(script.clone());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
        let socket = dir.path().join("upstream.sock");
        let unix = tokio::net::UnixListener::bind(&socket).unwrap();
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o666)).unwrap();
        let unix_app = app.clone();
        let unix_server = tokio::spawn(async move { axum::serve(unix, unix_app).await.unwrap() });
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Self {
            port,
            script,
            server,
            socket,
            unix_server,
            _dir: dir,
        }
    }

    pub fn release(&self) {
        self.script.release.notify_one();
    }

    pub fn requests(&self) -> Vec<Value> {
        self.script.requests.lock().unwrap().clone()
    }
}

impl Drop for Upstream {
    fn drop(&mut self) {
        self.server.abort();
        self.unix_server.abort();
    }
}

async fn unavailable() -> Response {
    (StatusCode::SERVICE_UNAVAILABLE, Json(json!({"error":{
        "message":"connection refused by scripted upstream", "type":"server_error", "code":"unavailable"
    }}))).into_response()
}

async fn chat(
    State(script): State<Script>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let key = headers.get("authorization").and_then(|v| v.to_str().ok());
    if key == Some("Bearer rejected-key") {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error":{
                "message":"Unauthorized", "type":"authentication_error", "code":"invalid_api_key"
            }})),
        )
            .into_response();
    }
    if !matches!(key, Some("Bearer contract-key" | "Bearer rotated-key")) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error":{"message":"Unauthorized"}})),
        )
            .into_response();
    }
    if body["model"] != "contract-text" {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error":{"message":"model not found"}})),
        )
            .into_response();
    }
    script.requests.lock().unwrap().push(body.clone());
    let messages = body["messages"].as_array().unwrap();
    let last = messages.last().unwrap();
    let message = if last["role"] == "tool" {
        json!({"role":"assistant","content":"Paris: 21 C"})
    } else if body["tools"]
        .as_array()
        .is_some_and(|tools| !tools.is_empty())
    {
        json!({"role":"assistant","content":null,"tool_calls":[{
            "id":"call_weather", "type":"function", "function":{"name":"weather", "arguments":"{\"city\":\"Paris\"}"}
        }]})
    } else {
        json!({"role":"assistant","content":if key == Some("Bearer rotated-key") { "rotated reply" } else { "contract reply" }})
    };
    let finish = if message.get("tool_calls").is_some() {
        "tool_calls"
    } else {
        "stop"
    };
    if body["stream"] == true {
        return streaming(
            message,
            finish,
            last["content"] == "gated stream",
            script.release,
        );
    }
    Json(json!({
        "id":"chatcmpl-contract", "object":"chat.completion", "created":0, "model":"contract-text",
        "choices":[{"index":0,"message":message,"finish_reason":finish}],
        "usage":{"prompt_tokens":10,"completion_tokens":4,"total_tokens":14}
    }))
    .into_response()
}

fn event(delta: Value, finish: Value) -> Bytes {
    Bytes::from(format!(
        "data: {}\n\n",
        json!({
            "id":"chatcmpl-contract", "object":"chat.completion.chunk", "created":0, "model":"contract-text",
            "choices":[{"index":0,"delta":delta,"finish_reason":finish}]
        })
    ))
}

fn streaming(message: Value, finish: &str, gated: bool, release: Arc<Notify>) -> Response {
    let deltas = if let Some(calls) = message.get("tool_calls") {
        vec![
            json!({"role":"assistant","tool_calls":[{"index":0,"id":calls[0]["id"],"type":"function",
                "function":{"name":"weather","arguments":"{\"city\":"}}]}),
            json!({"tool_calls":[{"index":0,"function":{"arguments":"\"Paris\"}"}}]}),
        ]
    } else {
        let content = message["content"].as_str().unwrap();
        let (first, second) = content.split_once(' ').unwrap();
        vec![
            json!({"role":"assistant","content":format!("{first} ")}),
            json!({"content":second}),
        ]
    };
    let first = event(deltas[0].clone(), Value::Null);
    let rest = [
        event(deltas[1].clone(), Value::Null),
        event(json!({}), json!(finish)),
        Bytes::from_static(b"data: [DONE]\n\n"),
    ];
    let initial = futures_util::stream::once(async { Ok::<_, Infallible>(first) });
    let later = futures_util::stream::once(async move {
        if gated {
            release.notified().await;
        }
        Ok::<_, Infallible>(Bytes::from(rest.concat()))
    });
    (
        [("content-type", "text/event-stream")],
        Body::from_stream(initial.chain(later)),
    )
        .into_response()
}
