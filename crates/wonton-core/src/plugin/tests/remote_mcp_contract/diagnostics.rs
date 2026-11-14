use rmcp::model::CallToolRequestParams;
use serde_json::{Value, json};
use tracing::instrument::WithSubscriber;

use super::capture_logs;
use super::fixture::{Bed, ErrorReply, REMOTE, SECRET, TEMPLATE, id, remote};
use crate::plugin::PluginPackage;
use crate::plugin::tests::langs;

#[tokio::test]
async fn remote_mcp_runtime_errors_are_redacted_before_reaching_the_agent() {
    for reply in [ErrorReply::Json, ErrorReply::Sse, ErrorReply::Http] {
        for method in ["initialize", "tools/list", "tools/call"] {
            let mut bed = Bed::new(true).await;
            bed.install("header").await.unwrap();
            *bed.fake.rpc_failure.lock().unwrap() = Some((method, reply));
            let (logs, subscriber) = capture_logs();
            let output = async {
                let agent = bed.agent().await;
                agent.list_tools(None).await.unwrap();
                let name = if method == "tools/call" {
                    format!("{}__search", id(REMOTE).slug())
                } else {
                    "wonton__list_plugins".to_owned()
                };
                let result = agent
                    .call_tool(CallToolRequestParams::new(name))
                    .await
                    .unwrap();
                if method == "tools/call" {
                    assert_eq!(result.is_error, Some(true), "{reply:?}: {result:?}");
                }
                let output = serde_json::to_string(&result).unwrap();
                agent.cancel().await.unwrap();
                output
            }
            .with_subscriber(subscriber)
            .await;
            assert!(output.contains(TEMPLATE), "{reply:?} {method}: {output}");
            assert!(output.contains(&format!("operation {method}")), "{output}");
            if matches!(reply, ErrorReply::Http) {
                assert!(output.contains("502"), "{output}");
            } else {
                assert!(output.contains("-32001"), "{output}");
            }
            let logs = String::from_utf8(logs.lock().unwrap().clone()).unwrap();
            let audit = std::fs::read_to_string(bed.data.path().join("audit.jsonl")).unwrap();
            for text in [&output, &logs, &audit] {
                assert!(!text.contains(SECRET), "{reply:?} {method}: {text}");
                assert!(
                    !text.contains("old%2Fone%3Fx%3D1"),
                    "{reply:?} {method}: {text}"
                );
            }
            assert!(bed.fake.calls.lock().unwrap().iter().any(|(uri, headers)| {
                uri.contains("old%2Fone%3Fx%3D1")
                    && headers["authorization"] == format!("Bearer {SECRET}")
            }));
        }
    }
}

#[tokio::test]
async fn remote_mcp_overlapping_credentials_never_leave_longer_key_suffixes() {
    for (short, long) in [("sk", "sk_live_abc123"), ("prefix/", "prefix/private?x=1")] {
        let bed = Bed::new(true).await;
        let template = "https://mcp.example.com/mcp/{secret:b}?first={secret:a}";
        let mut document: Value = serde_json::from_str(&remote("header").source).unwrap();
        document["x-wonton"]["remote_mcp"]["url"] = json!(template);
        document["x-wonton"]["remote_mcp"]["headers"] =
            json!({"Authorization": "Bearer {secret:b}"});
        document["x-wonton"]["secrets"] = json!({
            "a": {"input": {"title": {"en": "First key"}}},
            "b": {"input": {"title": {"en": "Second key"}}}
        });
        let package = PluginPackage::parse(&document.to_string(), &langs()).unwrap();
        bed.manager
            .secrets_for_test(&id(REMOTE), &[("a", short), ("b", long)]);
        *bed.fake.outcome.lock().unwrap() = "secret_rejected".into();
        let probe = bed
            .manager
            .install_package(package.clone(), &[], langs())
            .await
            .unwrap_err();
        assert_eq!(probe.kind(), "invalid_input");
        *bed.fake.outcome.lock().unwrap() = "ok".into();
        bed.manager
            .install_package(package, &[], langs())
            .await
            .unwrap();
        *bed.fake.rpc_failure.lock().unwrap() = Some(("tools/list", ErrorReply::Json));
        let upstreams = crate::mcp::upstream::Upstreams::new();
        let runtime = upstreams
            .tools(&bed.manager.mcp_servers().pop().unwrap())
            .await
            .unwrap_err();
        let encoded_short = percent_encoding::utf8_percent_encode(
            short,
            crate::plugin::proxy_auth::URL_SECRET_COMPONENT,
        )
        .to_string();
        let encoded_long = percent_encoding::utf8_percent_encode(
            long,
            crate::plugin::proxy_auth::URL_SECRET_COMPONENT,
        )
        .to_string();
        for output in [probe.to_string(), runtime] {
            assert!(output.contains(template), "{output}");
            for private in [
                short,
                long,
                long.strip_prefix(short).unwrap(),
                &encoded_short,
                &encoded_long,
                encoded_long.strip_prefix(&encoded_short).unwrap(),
            ] {
                assert!(!output.contains(private), "{output}");
            }
        }
    }
}
