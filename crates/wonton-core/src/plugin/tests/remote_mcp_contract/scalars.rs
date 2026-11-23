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
    secret: String,
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
        assert_eq!(
            forwarded["authorization"],
            format!("Bearer {}", case.secret)
        );
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

/// 跑一次真实工具调用：返回 Agent 看到的失败和审计文件，确认凭据还是原文发出去的。
async fn run(secret: &str, reply: ErrorReply, data: Value) -> (String, String) {
    let case = Arc::new(ScalarCase {
        secret: secret.to_owned(),
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
        .secrets_for_test(&id(REMOTE), &[("token", secret)]);
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
    agent.cancel().await.unwrap();
    (output, audit)
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
            let (output, audit) = run("123456", reply, data).await;
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
        }
    }
}

/// 数字的几种写法都要脱敏，而且短标识、负数、小数、大整数和数组里的值都算。
#[tokio::test]
async fn remote_mcp_numeric_error_scalars_keep_their_non_secret_shape() {
    for (secret, data, visible_shape) in [
        ("42", json!(-42), "-[secret]"),
        ("123456", json!(123456.5), "[secret].5"),
        (
            "123456789012345678",
            json!(123456789012345678u64),
            "[secret]",
        ),
        ("123456", json!([1, 123456, {"n": 123456}]), "[secret]"),
    ] {
        let (output, audit) = run(secret, ErrorReply::Json, data).await;
        for visible in [&output, &audit] {
            assert!(visible.contains("[secret]"), "{visible}");
            assert!(!visible.contains(secret), "{visible}");
        }
        assert!(output.contains(visible_shape), "{output}");
        assert!(
            output.contains("-32001"),
            "protocol error code changed: {output}"
        );
    }
}
