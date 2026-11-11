use std::collections::BTreeMap;
use std::time::Duration;

use futures_util::StreamExt;
use reqwest::StatusCode;
use serde_json::{Value, json};
use sse_stream::SseStream;

use super::{Bed, upstream::Upstream};

type Env = BTreeMap<String, String>;

fn request(env: &Env, content: Value, stream: bool) -> Value {
    json!({"model":env["WONTON_MODEL_DEFAULT"],"messages":[
        {"role":"system","content":"You are a test assistant."},
        {"role":"user","content":content}
    ],"stream":stream,"max_completion_tokens":32,"reasoning_effort":"low","service_tier":"default",
      "response_format":{"type":"json_schema","json_schema":{"name":"contract","strict":true,
        "schema":{"type":"object","properties":{"answer":{"type":"string"}},"required":["answer"],"additionalProperties":false}}}})
}

pub(super) async fn catalog(bed: &Bed, env: &Env) {
    let response = bed
        .http
        .get(format!("{}/models", env["WONTON_MODEL_BASE_URL"]))
        .bearer_auth(&env["WONTON_MODEL_API_KEY"])
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let models: Value = response.json().await.unwrap();
    assert!(
        models["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|model| model["id"] == env["WONTON_MODEL_DEFAULT"])
    );
}

pub(super) async fn wait_for_reply(bed: &Bed, env: &Env, expected: &str) {
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let response = bed.chat(env, request(env, json!("hello"), false)).await;
            if response.status() == StatusCode::OK {
                let body: Value = response.json().await.unwrap();
                if body["choices"][0]["message"]["content"] == expected {
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("declared default model never became usable");
}

pub(super) async fn exercise(bed: &Bed, env: &Env, upstream: &Upstream) {
    let port = bed.manager.cfg.gateway_port;
    let spec = &bed.target.spec;
    assert_eq!(
        env["WONTON_MODEL_BASE_URL"],
        format!(
            "http://models.wonton.internal:{}{}",
            port,
            spec.apis.openai.trim_end_matches('/')
        )
    );
    assert_eq!(env["WONTON_MODEL_DEFAULT"], spec.default_model);
    for (path, name) in [
        (&spec.apis.anthropic, "WONTON_MODEL_ANTHROPIC_BASE_URL"),
        (&spec.apis.gemini, "WONTON_MODEL_GEMINI_BASE_URL"),
    ] {
        if let Some(path) = path {
            assert_eq!(
                env[name],
                format!(
                    "http://models.wonton.internal:{}{}",
                    port,
                    path.trim_end_matches('/')
                )
            );
        }
    }
    wait_for_reply(bed, env, "contract reply").await;
    catalog(bed, env).await;
    for key in [None, Some("not-issued")] {
        let mut call = bed
            .http
            .get(format!("{}/models", env["WONTON_MODEL_BASE_URL"]));
        if let Some(key) = key {
            call = call.bearer_auth(key);
        }
        assert_eq!(
            call.send().await.unwrap().status(),
            StatusCode::UNAUTHORIZED
        );
        let mut call = bed
            .http
            .post(format!("{}/chat/completions", env["WONTON_MODEL_BASE_URL"]))
            .json(&request(env, json!("unauthorized"), false));
        if let Some(key) = key {
            call = call.bearer_auth(key);
        }
        assert_eq!(
            call.send().await.unwrap().status(),
            StatusCode::UNAUTHORIZED
        );
    }
    let response = bed.chat(env, request(env, json!("hello"), false)).await;
    assert_eq!(response.status(), StatusCode::OK);
    let reply: Value = response.json().await.unwrap();
    assert_eq!(reply["choices"][0]["message"]["role"], "assistant");
    assert_eq!(reply["choices"][0]["message"]["content"], "contract reply");
    assert_eq!(reply["choices"][0]["finish_reason"], "stop");
    assert_eq!(reply["usage"]["total_tokens"], 14);
    assert!(upstream.requests().iter().any(|request| {
        request["response_format"]["type"] == "json_schema"
            && request["max_completion_tokens"] == 32
            && request["reasoning_effort"] == "low"
            && request["service_tier"] == "default"
    }));
    let mut missing = request(env, json!("hello"), false);
    missing["model"] = json!("does-not-exist");
    let response = bed.chat(env, missing).await;
    assert!(matches!(
        response.status(),
        StatusCode::NOT_FOUND | StatusCode::BAD_REQUEST
    ));
    assert!(response.json::<Value>().await.unwrap()["error"].is_object());
    stream(bed, env, upstream).await;
    tools(bed, env).await;
    image(bed, env, upstream).await;
}

async fn stream(bed: &Bed, env: &Env, upstream: &Upstream) {
    let response = bed
        .chat(env, request(env, json!("gated stream"), true))
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        response.headers()["content-type"]
            .to_str()
            .unwrap()
            .starts_with("text/event-stream")
    );
    let mut events = Box::pin(SseStream::from_bytes_stream(response.bytes_stream()));
    let mut text = String::new();
    tokio::time::timeout(Duration::from_secs(5), async {
        while text.is_empty() {
            let event = events.next().await.unwrap().unwrap();
            let body: Value = serde_json::from_str(event.data.as_deref().unwrap()).unwrap();
            if let Some(delta) = body["choices"][0]["delta"]["content"].as_str() {
                text.push_str(delta);
            }
        }
    })
    .await
    .expect("gateway buffered the stream until completion");
    assert_eq!(text, "contract ");
    upstream.release();
    let mut finished = false;
    let mut done = false;
    while let Some(event) = events.next().await {
        let data = event.unwrap().data.unwrap();
        if data == "[DONE]" {
            done = true;
            break;
        }
        let body: Value = serde_json::from_str(&data).unwrap();
        if let Some(delta) = body["choices"][0]["delta"]["content"].as_str() {
            text.push_str(delta);
        }
        finished |= body["choices"][0]["finish_reason"] == "stop";
    }
    assert!(finished && done);
    assert_eq!(text, "contract reply");
}

async fn tools(bed: &Bed, env: &Env) {
    let mut body = request(env, json!("weather in Paris"), false);
    body["tools"] = json!([{"type":"function","function":{"name":"weather","description":"Get weather","parameters":{"type":"object","properties":{"city":{"type":"string"}},"required":["city"]}}}]);
    body["tool_choice"] = json!("auto");
    let response = bed.chat(env, body.clone()).await;
    assert_eq!(response.status(), StatusCode::OK);
    let reply: Value = response.json().await.unwrap();
    assert_eq!(reply["choices"][0]["finish_reason"], "tool_calls");
    let assistant = &reply["choices"][0]["message"];
    let call = &assistant["tool_calls"][0];
    assert_eq!(call["type"], "function");
    assert_eq!(call["function"]["name"], "weather");
    assert_eq!(
        serde_json::from_str::<Value>(call["function"]["arguments"].as_str().unwrap()).unwrap(),
        json!({"city":"Paris"})
    );
    let messages = body["messages"].as_array_mut().unwrap();
    messages.push(assistant.clone());
    messages.push(json!({"role":"tool","tool_call_id":call["id"],"content":"21 C"}));
    let response = bed.chat(env, body.clone()).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.json::<Value>().await.unwrap()["choices"][0]["message"]["content"],
        "Paris: 21 C"
    );
    body["messages"].as_array_mut().unwrap().truncate(2);
    body["stream"] = json!(true);
    let response = bed.chat(env, body).await;
    assert_eq!(response.status(), StatusCode::OK);
    let mut events = Box::pin(SseStream::from_bytes_stream(response.bytes_stream()));
    let (mut id, mut name, mut args) = (String::new(), String::new(), String::new());
    let mut finished = false;
    let mut done = false;
    while let Some(event) = events.next().await {
        let data = event.unwrap().data.unwrap();
        if data == "[DONE]" {
            done = true;
            break;
        }
        let body: Value = serde_json::from_str(&data).unwrap();
        let call = &body["choices"][0]["delta"]["tool_calls"][0];
        if let Some(value) = call["id"].as_str() {
            id.push_str(value);
        }
        if let Some(value) = call["function"]["name"].as_str() {
            name.push_str(value);
        }
        if let Some(value) = call["function"]["arguments"].as_str() {
            args.push_str(value);
        }
        finished |= body["choices"][0]["finish_reason"] == "tool_calls";
    }
    assert!(finished && done);
    assert_eq!(id, "call_weather");
    assert_eq!(name, "weather");
    assert_eq!(
        serde_json::from_str::<Value>(&args).unwrap(),
        json!({"city":"Paris"})
    );
}

async fn image(bed: &Bed, env: &Env, upstream: &Upstream) {
    let image = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+a9foAAAAASUVORK5CYII=";
    let content = json!([{"type":"text","text":"describe this"},{"type":"image_url","image_url":{"url":image}}]);
    let response = bed.chat(env, request(env, content.clone(), false)).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.json::<Value>().await.unwrap()["choices"][0]["message"]["content"],
        "contract reply"
    );
    let seen = upstream.requests();
    let messages = seen.last().unwrap()["messages"].as_array().unwrap();
    assert_eq!(messages.last().unwrap()["content"], content);
}
