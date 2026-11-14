use axum::body::to_bytes;
use serde_json::{Value, json};

use crate::plugin::tests::remote_mcp_contract::fixture::{Bed, REMOTE, SECRET, id};

#[tokio::test]
async fn remote_mcp_install_rejected_by_probe_outcome() {
    for (outcome, expected_status, expected_kind) in [
        ("needs_oauth", 400, "invalid_input"),
        ("needs_header_secret", 400, "invalid_input"),
        ("secret_rejected", 400, "invalid_input"),
        ("not_mcp", 400, "invalid_input"),
        ("blocked_address", 400, "invalid_input"),
        ("unreachable", 503, "remote_unavailable"),
        ("proxy_unavailable", 422, "egress_proxy_unavailable"),
        ("remote_error", 502, "remote_error"),
        ("unknown", 502, "upstream_unavailable"),
        ("malformed", 502, "upstream_unavailable"),
        ("http500", 502, "upstream_unavailable"),
    ] {
        let bed = Bed::new(true).await;
        *bed.fake.outcome.lock().unwrap() = outcome.into();
        let error = bed.install("header").await.unwrap_err();
        let response = super::api::install_error(&error);
        assert_eq!(response.status().as_u16(), expected_status, "{outcome}");
        let bytes = to_bytes(response.into_body(), 1 << 20).await.unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["error"]["kind"], expected_kind, "{outcome}: {body}");
        if outcome == "secret_rejected" {
            assert_eq!(body["error"]["problem"]["secrets"], json!(["token"]));
        }
        assert!(!body.to_string().contains(SECRET));
        assert!(!body.to_string().contains("old%2Fone%3Fx%3D1"));
        assert!(
            bed.manager.record_for_test(&id(REMOTE)).is_none(),
            "{outcome}"
        );
        assert!(bed.manager.mcp_servers().is_empty(), "{outcome}");
        assert_eq!(bed.docker.containers(), [bed.provider_container()]);
    }
}
