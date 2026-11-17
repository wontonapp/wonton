use std::time::Duration;

use axum::http::{Method, StatusCode};
use serde_json::json;

use super::probe_support::{Options, Server, config, initialize, probe, simple};

#[tokio::test]
async fn probe_skips_empty_control_events_before_initialize_and_tools() {
    let remote = Server::remote(Options {
        sse: true,
        empty_events: 2,
        ..Options::default()
    })
    .await;
    let result = simple(&remote).await;
    assert_eq!(result["outcome"], "ok", "{result}");
    assert_eq!(result["tools"], json!(["search"]));
    let requests = remote.requests();
    assert_eq!(requests.len(), 4);
    assert_eq!(requests.last().unwrap().method, Method::DELETE);
}

#[tokio::test]
async fn probe_sse_limit_is_not_a_lifetime_counter() {
    let remote = Server::remote(Options {
        sse: true,
        // Each valid control event is small; together these exceed 1 MiB.
        empty_events: 50_000,
        ..Options::default()
    })
    .await;
    let result = simple(&remote).await;
    assert_eq!(result["outcome"], "ok", "{result}");
    assert_eq!(result["tools"], json!(["search"]));
    assert_eq!(remote.requests().last().unwrap().method, Method::DELETE);
}

#[tokio::test]
async fn probe_html_post_errors_try_legacy_get_first() {
    for status in [
        StatusCode::BAD_REQUEST,
        StatusCode::NOT_FOUND,
        StatusCode::METHOD_NOT_ALLOWED,
    ] {
        let remote = Server::remote(Options {
            status,
            html: true,
            get_body: "event: endpoint\ndata: /messages?token=private-endpoint-secret\n\n"
                .to_owned(),
            ..Options::default()
        })
        .await;
        let result = probe(
            &remote.url,
            config(),
            "header",
            json!({"Authorization":"Bearer private-header-secret"}),
        )
        .await;
        assert_eq!(result["outcome"], "legacy_sse", "{status}: {result}");
        assert_eq!(result["http_status"], status.as_u16());
        for secret in [
            "private-endpoint-secret",
            "private-header-secret",
            "private-body-secret",
        ] {
            assert!(!result.to_string().contains(secret));
        }
        let requests = remote.requests();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[1].method, Method::GET);
        assert_eq!(requests[1].headers["accept"], "text/event-stream");
        assert_eq!(
            requests[1].headers["authorization"],
            "Bearer private-header-secret"
        );
        assert!(!requests[1].headers.contains_key("mcp-session-id"));
    }
}

#[tokio::test]
async fn probe_answers_ping_before_initialize_and_tools_results_are_released() {
    for id in [
        json!("health/private-id-secret"),
        json!(-73),
        json!(u64::MAX),
    ] {
        let remote = Server::remote(Options {
            sse: true,
            empty_events: 1,
            server_request: Some(json!({"jsonrpc":"2.0","id":id,"method":"ping"})),
            ..Options::default()
        })
        .await;
        let result = probe(
            &remote.url,
            config(),
            "header",
            json!({"Authorization":"Bearer private-header-secret"}),
        )
        .await;
        assert_eq!(result["outcome"], "ok", "{result}");
        assert_eq!(result["tools"], json!(["search"]));
        assert!(!result.to_string().contains("private-header-secret"));
        assert!(!result.to_string().contains("private-id-secret"));
        let requests = remote.requests();
        assert_eq!(requests.len(), 6);
        assert_eq!(requests[0].body["method"], "initialize");
        assert_eq!(
            requests[1].body,
            json!({"jsonrpc":"2.0","id":id,"result":{}})
        );
        assert!(!requests[1].headers.contains_key("mcp-protocol-version"));
        assert_eq!(requests[2].body["method"], "notifications/initialized");
        assert_eq!(requests[3].body["method"], "tools/list");
        assert_eq!(requests[4].body, requests[1].body);
        assert_eq!(requests[5].method, Method::DELETE);
        for request in &requests {
            assert_eq!(
                request.headers["authorization"],
                "Bearer private-header-secret"
            );
        }
        for request in &requests[1..] {
            assert_eq!(request.headers["mcp-session-id"], "probe-session");
        }
        for request in &requests[2..] {
            assert_eq!(request.headers["mcp-protocol-version"], "2025-06-18");
        }
    }
}

#[tokio::test]
async fn probe_replies_method_not_found_to_unsupported_server_requests() {
    let remote = Server::remote(Options {
        sse: true,
        server_request: Some(json!({
            "jsonrpc":"2.0","id":"private-request-secret","method":"sampling/createMessage",
            "params":{"messages":[]}
        })),
        ..Options::default()
    })
    .await;
    let result = simple(&remote).await;
    assert_eq!(result["outcome"], "ok", "{result}");
    assert!(!result.to_string().contains("private-request-secret"));
    let requests = remote.requests();
    assert_eq!(requests.len(), 6);
    for index in [1, 4] {
        assert_eq!(
            requests[index].body,
            json!({
                "jsonrpc":"2.0","id":"private-request-secret",
                "error":{"code":-32601,"message":"Method not found"}
            })
        );
        assert!(requests[index].body.get("result").is_none());
    }
    assert_eq!(requests[5].method, Method::DELETE);
}

#[tokio::test]
async fn probe_rejected_ping_reply_fails_safely_and_cleans_session() {
    let remote = Server::remote(Options {
        sse: true,
        server_request: Some(json!({"jsonrpc":"2.0","id":"private-id-secret","method":"ping"})),
        server_reply_status: StatusCode::FORBIDDEN,
        ..Options::default()
    })
    .await;
    let result = probe(
        &format!("{}?key=private-url-secret", remote.url),
        config(),
        "header",
        json!({"X-Api-Key":"private-header-secret"}),
    )
    .await;
    assert_eq!(result["outcome"], "remote_error", "{result}");
    assert_eq!(result["http_status"], 403);
    assert!(result.get("tools").is_none());
    for secret in [
        "private-id-secret",
        "private-url-secret",
        "private-header-secret",
        "private-reply-secret",
    ] {
        assert!(!result.to_string().contains(secret));
    }
    let requests = remote.requests();
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[1].body["result"], json!({}));
    assert_eq!(requests[2].method, Method::DELETE);
    for request in &requests[1..] {
        assert_eq!(request.headers["mcp-session-id"], "probe-session");
        assert_eq!(request.headers["x-api-key"], "private-header-secret");
    }
}

#[tokio::test]
async fn probe_ping_reply_uses_the_shared_handshake_deadline() {
    let remote = Server::remote(Options {
        sse: true,
        server_request: Some(json!({"jsonrpc":"2.0","id":"health","method":"ping"})),
        delay_method: Some("server-response"),
        ..Options::default()
    })
    .await;
    let mut config = config();
    config.probe_timeout = Duration::from_millis(150);
    let result = probe(&remote.url, config, "none", json!({})).await;
    assert_eq!(result["outcome"], "unreachable", "{result}");
    let requests = remote.requests();
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[1].body["id"], "health");
    assert_eq!(requests[2].method, Method::DELETE);
    assert_eq!(requests[2].headers["mcp-session-id"], "probe-session");
}

#[tokio::test]
async fn probe_ping_does_not_hide_incorrect_result_ids() {
    for tools_phase in [false, true] {
        let mut options = Options {
            sse: true,
            server_request: Some(json!({"jsonrpc":"2.0","id":"health","method":"ping"})),
            ..Options::default()
        };
        if tools_phase {
            options.tools["id"] = json!(77);
        } else {
            options.initialize = initialize();
            options.initialize["id"] = json!(77);
        }
        let remote = Server::remote(options).await;
        let result = simple(&remote).await;
        assert_eq!(result["outcome"], "remote_error", "{result}");
        assert!(result.get("tools").is_none());
        let requests = remote.requests();
        assert_eq!(requests.len(), if tools_phase { 6 } else { 3 });
        assert_eq!(
            requests[1].body,
            json!({"jsonrpc":"2.0","id":"health","result":{}})
        );
        assert_eq!(requests.last().unwrap().method, Method::DELETE);
    }
}

#[tokio::test]
async fn probe_malformed_server_messages_fail_instead_of_being_ignored() {
    for request in [
        json!({"jsonrpc":"1.0","id":"health","method":"ping"}),
        json!({"jsonrpc":"2.0","id":null,"method":"ping"}),
        json!({"jsonrpc":"2.0","id":[],"method":"ping"}),
        json!({"jsonrpc":"2.0","id":true,"method":"ping"}),
        json!({"jsonrpc":"2.0","id":"health","method":17}),
        json!({"jsonrpc":"2.0","id":"health","method":"ping","params":false}),
        json!({"jsonrpc":"2.0","id":"health","method":"ping","result":{}}),
    ] {
        let remote = Server::remote(Options {
            sse: true,
            server_request: Some(request),
            ..Options::default()
        })
        .await;
        let result = simple(&remote).await;
        assert_eq!(result["outcome"], "remote_error", "{result}");
        assert!(result.get("tools").is_none());
        let requests = remote.requests();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[1].method, Method::DELETE);
    }
}
