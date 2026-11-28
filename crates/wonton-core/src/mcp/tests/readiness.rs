//! 还没配好的插件：如实报缺什么，配好以后恢复。

use super::*;

/// 必填密钥没填的插件：工具不列出来、`tools_error` 点名缺的项，调用回 `not_ready`；填上以后恢复。
#[tokio::test]
async fn an_unconfigured_plugin_reports_not_ready_and_recovers_when_filled() {
    let source = NOTES.replace(
        "  mcp: {service: mcp, port: 8080, transport: streamable-http}",
        "  mcp: {service: mcp, port: 8080, transport: streamable-http, headers: {Authorization: 'Bearer {secret:token}'}}\n  \
         secrets:\n    token:\n      input:\n        title: {en: Token}\n",
    );
    let s = setup_with(
        Engine::unreachable_for_test(),
        Duration::from_secs(60),
        DEFAULT_WINDOW,
        Upstreams::new(),
        &source,
    )
    .await;
    let id = PluginId::parse("app.example.notes").unwrap();

    let names: Vec<String> = s
        .bus
        .tools(&s.caller)
        .await
        .into_iter()
        .map(|tool| tool.name.to_string())
        .collect();
    assert!(
        !names
            .iter()
            .any(|name| name.starts_with("app-example-notes__")),
        "{names:?}"
    );
    let problem = s.bus.upstreams.tools_problem(&id).unwrap();
    assert!(problem.contains("token"), "{problem}");

    let result = s.call("app-example-notes__search", json!({})).await;
    assert!(failed(&result));
    assert_eq!(
        super::tool_failure_tests::failure_kind(&result),
        Some("not_ready")
    );
    assert!(text(&result).contains("token"), "{}", text(&result));

    s.manager.secrets_for_test(&id, &[("token", "s3cret")]);
    let names: Vec<String> = s
        .bus
        .tools(&s.caller)
        .await
        .into_iter()
        .map(|tool| tool.name.to_string())
        .collect();
    assert!(
        names.contains(&"app-example-notes__search".to_owned()),
        "{names:?}"
    );
    let result = s.call("app-example-notes__search", json!({})).await;
    assert!(!failed(&result), "{}", text(&result));
}
