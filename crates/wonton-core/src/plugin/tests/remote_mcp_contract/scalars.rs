use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use base64::Engine as _;
use rmcp::model::CallToolRequestParams;
use serde_json::{Value, json};

use super::fixture::{Bed, ErrorReply, REMOTE, id};

struct ScalarCase {
    reply: ErrorReply,
    data: Value,
    calls: AtomicUsize,
}

async fn rpc(
    State(case): State<Arc<ScalarCase>>,
    headers: HeaderMap,
    axum::Json(request): axum::Json<Value>,
) -> Response {
    let method = request["method"].as_str().unwrap();
    if method == "tools/call" {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(headers["x-wonton-remote-forward"].as_bytes())
            .unwrap();
        let forwarded: BTreeMap<String, String> = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(forwarded["authorization"], "Bearer 123456");
        case.calls.fetch_add(1, Ordering::SeqCst);
        let error = json!({"jsonrpc": "2.0", "id": request["id"], "error": {
            "code": -32001, "message": "numeric-diagnostic-context", "data": case.data
        }});
        return match case.reply {
            ErrorReply::Json => (StatusCode::BAD_GATEWAY, axum::Json(error)).into_response(),
            ErrorReply::Sse => (
                [("content-type", "text/event-stream")],
                format!("data: {error}\n\n"),
            )
                .into_response(),
            ErrorReply::Http => (StatusCode::BAD_GATEWAY, error.to_string()).into_response(),
        };
    }
    let result = match method {
        "initialize" => json!({"protocolVersion": "2025-11-25", "capabilities": {"tools": {}},
            "serverInfo": {"name": "scalar-contract", "version": "1"}}),
        "tools/list" => json!({"tools": [{"name": "search", "inputSchema": {"type": "object"},
            "annotations": {"readOnlyHint": true}}]}),
        _ => return StatusCode::ACCEPTED.into_response(),
    };
    axum::Json(json!({"jsonrpc": "2.0", "id": request["id"], "result": result})).into_response()
}

#[tokio::test]
async fn remote_mcp_numeric_error_data_is_redacted_in_agent_failure_and_audit() {
    for reply in [ErrorReply::Json, ErrorReply::Sse] {
        for data in [
            json!(123456),
            json!({"token": 123456, "safe_counter": 907,
            "nested": [123456, true, null]}),
        ] {
            let nested = data.is_object();
            let case = Arc::new(ScalarCase {
                reply,
                data,
                calls: AtomicUsize::new(0),
            });
            let state = case.clone();
            let mut bed = Bed::with_connector(move |_, _| {
                axum::Router::new()
                    .route(
                        "/_wonton/probe",
                        post(|| async { axum::Json(json!({"outcome": "ok", "detail": ""})) }),
                    )
                    .route("/mcp", post(rpc))
                    .with_state(state)
            })
            .await;
            bed.manager
                .secrets_for_test(&id(REMOTE), &[("token", "123456")]);
            bed.install("header").await.unwrap();
            let agent = bed.agent().await;
            agent.list_tools(None).await.unwrap();
            let result = agent
                .call_tool(CallToolRequestParams::new(format!(
                    "{}__search",
                    id(REMOTE).slug()
                )))
                .await
                .unwrap();
            assert_eq!(result.is_error, Some(true), "{result:?}");
            assert_eq!(case.calls.load(Ordering::SeqCst), 1);
            let output = serde_json::to_string(&result).unwrap();
            let audit = std::fs::read_to_string(bed.data.path().join("audit.jsonl")).unwrap();
            for visible in [&output, &audit] {
                assert!(visible.contains("numeric-diagnostic-context"), "{visible}");
                assert!(visible.contains("[secret]"), "{visible}");
                assert!(!visible.contains("123456"), "{visible}");
                if nested {
                    assert!(visible.contains("907"), "{visible}");
                }
            }
            assert!(
                output.contains("-32001"),
                "protocol error code changed: {output}"
            );
            assert!(output.contains("upstream_error"), "{output}");
            agent.cancel().await.unwrap();
        }
    }
}
