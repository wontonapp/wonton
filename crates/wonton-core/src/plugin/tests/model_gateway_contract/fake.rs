use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{Value, json};
use tokio::net::TcpListener;

use crate::plugin::fake_docker::FakeDocker;
use crate::plugin::provides::{
    CredentialCommands, CustomProviderCommands, ModelApis, ModelsProvider, ProviderCommands,
};

#[derive(Clone)]
struct Provider {
    base: String,
    model: String,
    key: String,
}

#[derive(Default)]
struct Store {
    keys: BTreeMap<String, String>,
    configured: BTreeMap<String, Provider>,
    active: Option<String>,
    serial: usize,
}

type Shared = Arc<Mutex<Store>>;
const DEFAULT: &str = "opaque:default";
const BASE: &str = "/compat/openai";

struct Server(tokio::task::JoinHandle<()>);
impl Drop for Server {
    fn drop(&mut self) {
        self.0.abort();
    }
}

pub(super) async fn start(upstream_port: u16) -> super::Target {
    let shared = Shared::default();
    let app = Router::new()
        .route(&format!("{BASE}/models"), get(models))
        .route(&format!("{BASE}/chat/completions"), post(chat))
        .route(&format!("{BASE}/responses"), post(chat))
        .with_state(shared.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream = listener.local_addr().unwrap().to_string();
    let guard = Server(tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap()
    }));
    let (docker, engine) = FakeDocker::start().await;
    docker.on_exec_output(move |_, cmd, input| command(&shared, cmd, input));
    super::Target {
        spec: ModelsProvider {
            service: "gateway".into(),
            port: 8989,
            apis: ModelApis {
                openai: BASE.into(),
                anthropic: Some("/compat/messages".into()),
                gemini: Some("/compat/generative".into()),
            },
            default_model: DEFAULT.into(),
            credentials: CredentialCommands {
                issue: vec!["credential-create".into(), "{name}".into()],
                revoke: vec!["credential-delete".into(), "{name}".into()],
                service: None,
            },
            providers: Some(ProviderCommands {
                list: vec!["catalog".into()],
                save: vec!["persist".into(), "{id}".into()],
                test: vec!["probe".into(), "{id}".into()],
                custom: Some(CustomProviderCommands {
                    save: vec!["persist-custom".into()],
                    test: vec!["probe-custom".into()],
                }),
            }),
        },
        upstream,
        backend_url: format!("http://127.0.0.1:{upstream_port}/v1"),
        docker,
        engine,
        _guard: Box::new(guard),
    }
}

fn command(shared: &Shared, argv: &[String], input: &[u8]) -> (i64, String, String) {
    let text = String::from_utf8_lossy(input);
    match argv
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["credential-create", name] => {
            shared
                .lock()
                .unwrap()
                .keys
                .entry((*name).into())
                .or_insert_with(|| text.trim_end().into());
        }
        ["credential-delete", name] => {
            shared.lock().unwrap().keys.remove(*name);
        }
        ["catalog"] => {
            let store = shared.lock().unwrap();
            let mut presets = vec![json!({"id":"offline","title":"Offline provider"})];
            presets.extend(
                store
                    .configured
                    .keys()
                    .filter(|id| *id != "offline")
                    .map(|id| json!({"id":id,"title":id})),
            );
            return (0, json!({"presets":presets,"configured":store.configured.keys().collect::<Vec<_>>()} ).to_string(), String::new());
        }
        [action @ ("persist-custom" | "probe-custom")] => {
            let body: Value = serde_json::from_slice(input).unwrap();
            let provider = Provider {
                base: body["base_url"].as_str().unwrap().into(),
                model: body["model"].as_str().unwrap().into(),
                key: body["key"].as_str().unwrap().into(),
            };
            let code = probe(&provider);
            if *action == "persist-custom" && matches!(code, 0 | 4) {
                let mut store = shared.lock().unwrap();
                store.serial += 1;
                let id = format!("custom-{}", store.serial);
                store.configured.insert(id.clone(), provider);
                store.active = Some(id);
            }
            return (code, String::new(), String::new());
        }
        [action @ ("persist" | "probe"), id] => {
            let existing = shared.lock().unwrap().configured.get(*id).cloned();
            if *id == "offline" {
                if *action == "persist" {
                    shared.lock().unwrap().configured.insert(
                        (*id).into(),
                        Provider {
                            base: "http://offline.invalid".into(),
                            model: "offline".into(),
                            key: text.trim_end().into(),
                        },
                    );
                }
                return (4, String::new(), String::new());
            }
            let Some(mut provider) = existing else {
                return (2, String::new(), String::new());
            };
            provider.key = text.trim_end().into();
            let code = probe(&provider);
            if *action == "persist" && matches!(code, 0 | 4) {
                shared
                    .lock()
                    .unwrap()
                    .configured
                    .insert((*id).into(), provider);
            }
            return (code, String::new(), String::new());
        }
        _ => return (2, String::new(), String::new()),
    }
    (0, String::new(), String::new())
}

fn probe(provider: &Provider) -> i64 {
    tokio::task::block_in_place(|| {
        tokio::runtime::Handle::current().block_on(async {
        let response = reqwest::Client::builder().no_proxy().build().unwrap()
            .post(format!("{}/chat/completions", provider.base)).bearer_auth(&provider.key)
            .json(&json!({"model":provider.model,"messages":[{"role":"user","content":"probe"}],"stream":false}))
            .send().await;
        match response.map(|r| r.status()) {
            Ok(StatusCode::OK) => 0,
            Ok(StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) => 3,
            _ => 4,
        }
    })
    })
}

fn authorized(shared: &Shared, headers: &HeaderMap) -> bool {
    let key = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    key.is_some_and(|key| {
        shared
            .lock()
            .unwrap()
            .keys
            .values()
            .any(|value| value == key)
    })
}

async fn models(State(shared): State<Shared>, headers: HeaderMap) -> Response {
    if !authorized(&shared, &headers) {
        return error(StatusCode::UNAUTHORIZED);
    }
    let listed = shared.lock().unwrap().active.is_some();
    Json(json!({"object":"list","data":if listed { json!([{"id":DEFAULT,"object":"model"}]) } else { json!([]) }})).into_response()
}

async fn chat(
    State(shared): State<Shared>,
    headers: HeaderMap,
    axum::extract::OriginalUri(uri): axum::extract::OriginalUri,
    Json(mut body): Json<Value>,
) -> Response {
    if !authorized(&shared, &headers) {
        return error(StatusCode::UNAUTHORIZED);
    }
    let provider = {
        let store = shared.lock().unwrap();
        store
            .active
            .as_ref()
            .and_then(|id| store.configured.get(id))
            .cloned()
    };
    let Some(provider) = provider.filter(|_| body["model"] == DEFAULT) else {
        return error(StatusCode::NOT_FOUND);
    };
    body["model"] = json!(provider.model);
    let response = reqwest::Client::builder()
        .no_proxy()
        .build()
        .unwrap()
        .post(format!(
            "{}{}",
            provider.base,
            uri.path().strip_prefix(BASE).unwrap()
        ))
        .bearer_auth(&provider.key)
        .json(&body)
        .send()
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    (status, headers, Body::from_stream(response.bytes_stream())).into_response()
}

fn error(status: StatusCode) -> Response {
    (
        status,
        Json(json!({"error":{"message":status.as_str(),"type":"invalid_request_error"}})),
    )
        .into_response()
}
