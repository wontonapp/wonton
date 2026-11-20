use serde_json::{Value, json};

use super::fixture::{Bed, REMOTE, id, remote};
use crate::plugin::PluginPackage;
use crate::plugin::tests::langs;

fn package(url: &str, required: bool) -> PluginPackage {
    let mut document: Value = serde_json::from_str(&remote("header").source).unwrap();
    document["x-wonton"]["remote_mcp"]["url"] = json!(url);
    document["x-wonton"]["secrets"]["token"]["input"]["required"] = json!(required);
    PluginPackage::parse(&document.to_string(), &langs()).unwrap()
}

#[tokio::test]
async fn remote_mcp_optional_header_can_be_omitted_filled_and_cleared() {
    let bed = Bed::new(true).await;
    bed.manager.secrets_for_test(&id(REMOTE), &[]);
    bed.manager
        .install_package(package("https://mcp.example.com/mcp", false), &[], langs())
        .await
        .unwrap();
    assert_eq!(bed.fake.probes.lock().unwrap()[0]["headers"], json!({}));
    let writes = bed.docker.writes();
    let upstreams = crate::mcp::upstream::Upstreams::new();
    for value in [None, Some("optional-filled-key"), None] {
        bed.manager
            .set_secret_input(&id(REMOTE), "token", value, &langs())
            .await
            .unwrap();
        let server = bed.manager.mcp_servers().pop().unwrap();
        upstreams
            .call(&server, "search", None, std::future::pending())
            .await
            .unwrap();
        let calls = bed.fake.calls.lock().unwrap();
        let (_, headers) = calls.last().unwrap();
        match value {
            Some(value) => assert_eq!(headers["authorization"], format!("Bearer {value}")),
            None => assert!(headers.get("authorization").is_none()),
        }
    }
    assert_eq!(bed.docker.writes(), writes);
}

#[tokio::test]
async fn remote_mcp_missing_required_header_or_url_input_is_not_sent() {
    for (url, required) in [
        ("https://mcp.example.com/mcp", true),
        ("https://mcp.example.com/mcp/{secret:token}", false),
    ] {
        let bed = Bed::new(true).await;
        bed.manager.secrets_for_test(&id(REMOTE), &[]);
        let error = bed
            .manager
            .install_package(package(url, required), &[], langs())
            .await
            .unwrap_err();
        assert_eq!(error.kind(), "invalid_input");
        assert!(bed.fake.probes.lock().unwrap().is_empty());
        assert!(bed.manager.record_for_test(&id(REMOTE)).is_none());
    }
}

#[tokio::test]
async fn remote_mcp_expanded_dot_segments_are_rejected_before_probe() {
    for (url, value) in [
        ("https://mcp.example.com/mcp/{secret:token}/end", "."),
        ("https://mcp.example.com/mcp/{secret:token}/end", ".."),
        ("https://mcp.example.com/mcp/.{secret:token}/end", "."),
        ("https://mcp.example.com/mcp/%2{secret:token}/end", "e"),
        ("https://mcp.example.com/mcp/{secret:token}%2E/end", "."),
    ] {
        let bed = Bed::new(true).await;
        bed.manager
            .secrets_for_test(&id(REMOTE), &[("token", value)]);
        let error = bed
            .manager
            .install_package(package(url, true), &[], langs())
            .await
            .unwrap_err();
        assert_eq!(error.kind(), "invalid_input");
        assert!(bed.fake.probes.lock().unwrap().is_empty());
        assert!(bed.manager.record_for_test(&id(REMOTE)).is_none());
    }
}

#[tokio::test]
async fn remote_mcp_dots_inside_segments_or_queries_keep_the_original_target() {
    for (url, value, expected) in [
        (
            "https://mcp.example.com/mcp/prefix-{secret:token}/end",
            "..",
            "https://mcp.example.com/mcp/prefix-../end",
        ),
        (
            "https://mcp.example.com/mcp/{secret:token}/end",
            "%2e",
            "https://mcp.example.com/mcp/%252e/end",
        ),
        (
            "https://mcp.example.com/mcp/{secret:token}/end",
            "...",
            "https://mcp.example.com/mcp/.../end",
        ),
        (
            "https://mcp.example.com?key={secret:token}/../",
            ".",
            "https://mcp.example.com/?key=./../",
        ),
    ] {
        let bed = Bed::new(true).await;
        bed.manager
            .secrets_for_test(&id(REMOTE), &[("token", value)]);
        bed.manager
            .install_package(package(url, true), &[], langs())
            .await
            .unwrap();
        assert_eq!(bed.fake.probes.lock().unwrap()[0]["url"], expected);
        assert!(bed.manager.record_for_test(&id(REMOTE)).is_some());
    }
}
