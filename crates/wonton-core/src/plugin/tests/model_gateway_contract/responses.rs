use std::collections::BTreeMap;

use futures_util::StreamExt;
use reqwest::StatusCode;
use serde_json::{Value, json};
use sse_stream::SseStream;

use super::Bed;

type Env = BTreeMap<String, String>;

async fn send(bed: &Bed, env: &Env, body: Value) -> reqwest::Response {
    bed.http
        .post(format!("{}/responses", env["WONTON_MODEL_BASE_URL"]))
        .bearer_auth(&env["WONTON_MODEL_API_KEY"])
        .json(&body)
        .send()
        .await
        .unwrap()
}

fn request(env: &Env, stream: bool) -> Value {
    json!({"model":env["WONTON_MODEL_DEFAULT"],"instructions":"You are a test assistant.",
        "input":[{"role":"user","content":[{"type":"input_text","text":"hello"}]}],
        "stream":stream,"store":false,"max_output_tokens":32})
}

fn output_text(body: &Value) -> String {
    body["output"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["type"] == "message")
        .flat_map(|item| item["content"].as_array().unwrap())
        .filter_map(|part| part["text"].as_str())
        .collect()
}

pub(super) async fn exercise(bed: &Bed, env: &Env) {
    let response = send(bed, env, request(env, false)).await;
    assert_eq!(response.status(), StatusCode::OK);
    let reply: Value = response.json().await.unwrap();
    assert_eq!(reply["object"], "response");
    assert_eq!(reply["status"], "completed");
    assert_eq!(output_text(&reply), "contract reply");
    stream(bed, env).await;
    tools(bed, env).await;
}

async fn stream(bed: &Bed, env: &Env) {
    let response = send(bed, env, request(env, true)).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        response.headers()["content-type"]
            .to_str()
            .unwrap()
            .starts_with("text/event-stream")
    );
    let mut events = Box::pin(SseStream::from_bytes_stream(response.bytes_stream()));
    let mut text = String::new();
    let mut completed = false;
    while let Some(event) = events.next().await {
        let data = event.unwrap().data.unwrap();
        if data == "[DONE]" {
            break;
        }
        let event: Value = serde_json::from_str(&data).unwrap();
        match event["type"].as_str() {
            Some("response.output_text.delta") => text.push_str(event["delta"].as_str().unwrap()),
            Some("response.completed") => {
                assert_eq!(event["response"]["status"], "completed");
                assert_eq!(output_text(&event["response"]), "contract reply");
                completed = true;
            }
            _ => {}
        }
    }
    assert!(completed);
    assert_eq!(text, "contract reply");
}

async fn tools(bed: &Bed, env: &Env) {
    let mut body = request(env, false);
    body["tools"] = json!([{"type":"function","name":"weather","description":"Get weather",
        "parameters":{"type":"object","properties":{"city":{"type":"string"}},"required":["city"]}}]);
    body["tool_choice"] = json!("auto");
    let response = send(bed, env, body.clone()).await;
    assert_eq!(response.status(), StatusCode::OK);
    let reply: Value = response.json().await.unwrap();
    let call = reply["output"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["type"] == "function_call")
        .unwrap();
    assert_eq!(call["name"], "weather");
    assert_eq!(
        serde_json::from_str::<Value>(call["arguments"].as_str().unwrap()).unwrap(),
        json!({"city":"Paris"})
    );
    stream_body(bed, env, body.clone()).await;
    let input = body["input"].as_array_mut().unwrap();
    input.push(call.clone());
    input.push(json!({"type":"function_call_output","call_id":call["call_id"],"output":"21 C"}));
    let response = send(bed, env, body).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        output_text(&response.json::<Value>().await.unwrap()),
        "Paris: 21 C"
    );
}

async fn stream_body(bed: &Bed, env: &Env, mut body: Value) {
    body["stream"] = json!(true);
    let response = send(bed, env, body).await;
    assert_eq!(response.status(), StatusCode::OK);
    let mut events = Box::pin(SseStream::from_bytes_stream(response.bytes_stream()));
    let mut arguments = String::new();
    let mut call_id = None;
    let mut completed = false;
    while let Some(event) = events.next().await {
        let data = event.unwrap().data.unwrap();
        if data == "[DONE]" {
            break;
        }
        let event: Value = serde_json::from_str(&data).unwrap();
        match event["type"].as_str() {
            Some("response.output_item.added") if event["item"]["type"] == "function_call" => {
                assert_eq!(event["item"]["name"], "weather");
                call_id = Some(event["item"]["call_id"].as_str().unwrap().to_owned());
            }
            Some("response.function_call_arguments.delta") => {
                arguments.push_str(event["delta"].as_str().unwrap())
            }
            Some("response.completed") => completed = true,
            _ => {}
        }
    }
    assert!(completed);
    assert_eq!(call_id.as_deref(), Some("call_weather"));
    assert_eq!(
        serde_json::from_str::<Value>(&arguments).unwrap(),
        json!({"city":"Paris"})
    );
}
