use axum::Json;
use axum::body::Body;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};

pub(super) async fn responses(headers: HeaderMap, Json(body): Json<Value>) -> Response {
    if headers.get("authorization").and_then(|v| v.to_str().ok()) != Some("Bearer contract-key") {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error":{"message":"Unauthorized"}})),
        )
            .into_response();
    }
    let input = body["input"].as_array();
    let answered = input.is_some_and(|items| {
        items
            .iter()
            .any(|item| item["type"] == "function_call_output")
    });
    let tool = body["tools"]
        .as_array()
        .is_some_and(|tools| !tools.is_empty())
        && !answered;
    let output = if tool {
        json!([{"id":"fc_contract","type":"function_call","status":"completed","call_id":"call_weather","name":"weather","arguments":"{\"city\":\"Paris\"}"}])
    } else {
        json!([{"id":"msg_contract","type":"message","status":"completed","role":"assistant","content":[{"type":"output_text","text":if answered { "Paris: 21 C" } else { "contract reply" },"annotations":[]}]}])
    };
    let response = json!({"id":"resp_contract","object":"response","created_at":0,"status":"completed","error":null,
        "incomplete_details":null,"model":"contract-text","output":output,"parallel_tool_calls":true,
        "usage":{"input_tokens":10,"output_tokens":4,"total_tokens":14}});
    if body["stream"] != true {
        return Json(response).into_response();
    }
    let mut events = vec![
        json!({"type":"response.created","sequence_number":0,"response":{
            "id":"resp_contract","object":"response","created_at":0,"status":"in_progress","model":"contract-text","output":[]
        }}),
    ];
    if tool {
        let item = &output[0];
        events.extend([
            json!({"type":"response.output_item.added","sequence_number":1,"output_index":0,"item":{
                "id":"fc_contract","type":"function_call","status":"in_progress","call_id":"call_weather","name":"weather","arguments":""
            }}),
            json!({"type":"response.function_call_arguments.delta","sequence_number":2,"item_id":"fc_contract","output_index":0,"delta":"{\"city\":\"Paris\"}"}),
            json!({"type":"response.function_call_arguments.done","sequence_number":3,"item_id":"fc_contract","output_index":0,"arguments":"{\"city\":\"Paris\"}"}),
            json!({"type":"response.output_item.done","sequence_number":4,"output_index":0,"item":item}),
        ]);
    } else {
        let content = &output[0]["content"][0];
        events.extend([
            json!({"type":"response.output_item.added","sequence_number":1,"output_index":0,"item":{
                "id":"msg_contract","type":"message","status":"in_progress","role":"assistant","content":[]
            }}),
            json!({"type":"response.content_part.added","sequence_number":2,"item_id":"msg_contract","output_index":0,"content_index":0,"part":{"type":"output_text","text":"","annotations":[]}}),
            json!({"type":"response.output_text.delta","sequence_number":3,"item_id":"msg_contract","output_index":0,"content_index":0,"delta":content["text"]}),
            json!({"type":"response.output_text.done","sequence_number":4,"item_id":"msg_contract","output_index":0,"content_index":0,"text":content["text"]}),
            json!({"type":"response.content_part.done","sequence_number":5,"item_id":"msg_contract","output_index":0,"content_index":0,"part":content}),
            json!({"type":"response.output_item.done","sequence_number":6,"output_index":0,"item":output[0]}),
        ]);
    }
    events.push(json!({"type":"response.completed","sequence_number":7,"response":response}));
    let stream = futures_util::stream::iter(events.into_iter().map(|event| {
        Ok::<_, std::convert::Infallible>(format!(
            "event: {}\ndata: {event}\n\n",
            event["type"].as_str().unwrap()
        ))
    }));
    (
        [("content-type", "text/event-stream")],
        Body::from_stream(stream),
    )
        .into_response()
}
