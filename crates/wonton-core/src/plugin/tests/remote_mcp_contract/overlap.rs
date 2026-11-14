use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use axum::extract::State;
use axum::routing::post;
use serde_json::{Value, json};

use super::fixture::{Bed, ErrorReply, REMOTE, TEMPLATE, id, remote};
use crate::plugin::PluginPackage;
use crate::plugin::tests::langs;

async fn gated_probe(
    State(accept): State<Arc<AtomicBool>>,
    axum::Json(body): axum::Json<Value>,
) -> axum::Json<Value> {
    let raw = body["headers"]["authorization"].as_str().unwrap();
    let encoded =
        percent_encoding::utf8_percent_encode(raw, crate::plugin::proxy_auth::URL_SECRET_COMPONENT);
    axum::Json(
        json!({"outcome": if accept.load(Ordering::SeqCst) { "ok" } else { "secret_rejected" },
        "detail": format!("remote URL {} header {raw} encoded {encoded}", body["url"].as_str().unwrap())}),
    )
}

async fn rejected_details(inputs: &[(&str, &str)], url: &str, header: &str) -> [String; 2] {
    let accept = Arc::new(AtomicBool::new(false));
    let state = accept.clone();
    let bed = Bed::with_connector(move |_, routes| {
        routes.merge(
            axum::Router::new()
                .route("/_wonton/probe", post(gated_probe))
                .with_state(state),
        )
    })
    .await;
    let mut document: Value = serde_json::from_str(&remote("header").source).unwrap();
    document["x-wonton"]["remote_mcp"]["url"] = json!(url);
    document["x-wonton"]["remote_mcp"]["headers"] = json!({"Authorization": header});
    document["x-wonton"]["secrets"] = Value::Object(
        inputs
            .iter()
            .map(|(name, _)| {
                (
                    (*name).to_owned(),
                    json!({"input": {"title": {"en": "Credential"}}}),
                )
            })
            .collect(),
    );
    let package = PluginPackage::parse(&document.to_string(), &langs()).unwrap();
    bed.manager.secrets_for_test(&id(REMOTE), inputs);
    let probe = bed
        .manager
        .install_package(package.clone(), &[], langs())
        .await
        .unwrap_err();
    assert_eq!(probe.kind(), "invalid_input");
    assert!(bed.manager.record_for_test(&id(REMOTE)).is_none());
    accept.store(true, Ordering::SeqCst);
    bed.manager
        .install_package(package, &[], langs())
        .await
        .unwrap();
    *bed.fake.rpc_failure.lock().unwrap() = Some(("tools/list", ErrorReply::Json));
    let runtime = crate::mcp::upstream::Upstreams::new()
        .tools(&bed.manager.mcp_servers().pop().unwrap())
        .await
        .unwrap_err();
    [probe.to_string(), runtime]
}

#[tokio::test]
async fn remote_mcp_shifted_overlaps_hide_the_full_raw_and_encoded_sensitive_union() {
    let url = "https://mcp.example.com/mcp";
    for (prefix, token, tail, encoded_tail) in [
        (
            "Bearer sk_live_",
            "sk_live_abc123private999",
            "abc123private999",
            "abc123private999",
        ),
        (
            "Bearer prefix/",
            "prefix/private?x=1",
            "private?x=1",
            "private%3Fx%3D1",
        ),
    ] {
        let details = rejected_details(
            &[("a_prefix", prefix), ("token", token)],
            url,
            "Bearer {secret:token}",
        )
        .await;
        for visible in details {
            assert!(
                visible.contains(url),
                "non-sensitive target lost: {visible}"
            );
            assert!(!visible.contains(token), "{visible}");
            assert!(!visible.contains(tail), "{visible}");
            assert!(!visible.contains(encoded_tail), "{visible}");
        }
    }
}

#[tokio::test]
async fn remote_mcp_secret_overlaps_cannot_destroy_the_full_url_template_replacement() {
    let details = rejected_details(
        &[
            ("probe_prefix", "remote URL https://mcp.example.com"),
            ("rpc_prefix", "upstream https://mcp.example.com"),
            ("token", "private/key?x=1"),
        ],
        TEMPLATE,
        "Bearer {secret:token}",
    )
    .await;
    for visible in details {
        assert!(visible.contains(TEMPLATE), "{visible}");
        assert!(!visible.contains("private/key?x=1"), "{visible}");
        assert!(!visible.contains("private%2Fkey%3Fx%3D1"), "{visible}");
    }
}

#[tokio::test]
async fn remote_mcp_overlaps_with_the_same_secret_do_not_leave_a_suffix() {
    let details = rejected_details(
        &[("token", "abab")],
        "https://mcp.example.com/mcp",
        "Bearer {secret:token}ab",
    )
    .await;
    for visible in details {
        assert!(visible.contains("[secret]"), "{visible}");
        assert!(!visible.contains("abab"), "{visible}");
        assert!(!visible.contains("[secret]ab"), "{visible}");
    }
}
