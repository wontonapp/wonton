use super::*;
use crate::acp::test_support::*;

#[tokio::test]
async fn fake_agent_round_trip_sets_http_mcp_and_mode() {
    let agent = FakeAgent::start(Scenario::Normal).await;
    let client = bounded(AcpClient::connect(&agent.socket, broker()), "initialize")
        .await
        .unwrap();
    let session = bounded(
        client.new_session("/workspace", "http://mcp.wonton.internal/mcp", "plan"),
        "session/new",
    )
    .await
    .unwrap();
    assert_eq!(session, "session-1");
    let result = bounded(
        client.prompt("session-1", "hi", 1024, Duration::from_secs(2)),
        "prompt",
    )
    .await
    .unwrap();
    assert_eq!(result.text, "hello");
    assert_eq!(
        bounded(client.steer("session-1", "continue"), "steering")
            .await
            .unwrap(),
        "injected"
    );
    bounded(client.close("session-1"), "close").await.unwrap();
    let requests = agent.requests().await;
    let new_session = requests
        .iter()
        .find(|request| request["method"] == "session/new")
        .unwrap();
    assert_eq!(new_session["params"]["mcpServers"][0]["type"], "http");
    let mode = requests
        .iter()
        .find(|request| request["method"] == "session/set_mode")
        .unwrap();
    assert_eq!(mode["params"]["modeId"], "plan");
    drop(client);
    agent.finish().await;
}

#[tokio::test]
async fn agent_without_modes_gets_a_session_without_set_mode() {
    let agent = FakeAgent::start(Scenario::NoModes).await;
    let client = bounded(AcpClient::connect(&agent.socket, broker()), "initialize")
        .await
        .unwrap();
    let session = bounded(
        client.new_session("/workspace", "http://mcp.wonton.internal/mcp", "plan"),
        "session/new",
    )
    .await
    .unwrap();
    assert_eq!(session, "session-1");
    let requests = agent.requests().await;
    assert!(
        !requests
            .iter()
            .any(|request| request["method"] == "session/set_mode")
    );
    drop(client);
    agent.finish().await;
}

#[tokio::test]
async fn timeout_cancels_and_next_prompt_discards_late_update() {
    let agent = FakeAgent::start(Scenario::CancelLateUpdate).await;
    let client = bounded(AcpClient::connect(&agent.socket, broker()), "initialize")
        .await
        .unwrap();
    let first = bounded(
        client.prompt("session-1", "long", 1024, Duration::from_millis(50)),
        "cancelled prompt",
    )
    .await;
    assert!(matches!(first, Err(Error::TransportUnavailable(_))));
    let result = bounded(
        client.prompt("session-1", "new", 1024, Duration::from_secs(2)),
        "next prompt",
    )
    .await
    .unwrap();
    assert_eq!(result.text, "new");
    drop(client);
    agent.finish().await;
}

#[tokio::test]
async fn the_client_reports_an_agent_disconnect() {
    let agent = FakeAgent::start(Scenario::Normal).await;
    let client = bounded(AcpClient::connect(&agent.socket, broker()), "initialize")
        .await
        .unwrap();
    let mut closed = client.closed();
    assert!(!*closed.borrow());
    agent.finish().await;
    assert!(bounded(closed.changed(), "closed").await.is_ok());
    assert!(*closed.borrow());
}

#[tokio::test]
async fn restored_replay_updates_do_not_leak_into_the_next_prompt() {
    let agent = FakeAgent::start(Scenario::RestoreReplay).await;
    let client = bounded(AcpClient::connect(&agent.socket, broker()), "initialize")
        .await
        .unwrap();
    bounded(
        client.load_or_resume("session-1", "/workspace", "http://mcp.wonton.internal/mcp"),
        "session/load",
    )
    .await
    .unwrap();
    let result = bounded(
        client.prompt("session-1", "next", 1024, Duration::from_secs(2)),
        "prompt",
    )
    .await
    .unwrap();
    assert_eq!(result.text, "hello");
    assert!(!result.text.contains("回放"));
    drop(client);
    agent.finish().await;
}

#[tokio::test]
async fn missing_restore_support_is_treated_as_a_lost_session() {
    for scenario in [Scenario::RestoreUnsupported, Scenario::RestoreMethodMissing] {
        let agent = FakeAgent::start(scenario).await;
        let client = bounded(AcpClient::connect(&agent.socket, broker()), "initialize")
            .await
            .unwrap();
        let result = bounded(
            client.load_or_resume("session-1", "/workspace", "http://mcp.wonton.internal/mcp"),
            "session/load",
        )
        .await;
        assert!(matches!(result, Err(Error::SessionUnavailable(_))));
        drop(client);
        agent.finish().await;
    }
}

#[tokio::test]
async fn prompt_errors_keep_their_categories() {
    for (scenario, transport) in [
        (Scenario::PromptTransportError, true),
        (Scenario::PromptUpstreamError, false),
    ] {
        let agent = FakeAgent::start(scenario).await;
        let client = bounded(AcpClient::connect(&agent.socket, broker()), "initialize")
            .await
            .unwrap();
        let error = bounded(
            client.prompt("session-1", "hi", 1024, Duration::from_secs(2)),
            "prompt",
        )
        .await
        .unwrap_err();
        if transport {
            assert!(
                matches!(error, Error::TransportUnavailable(_)),
                "got {error}"
            );
        } else {
            assert!(matches!(error, Error::Upstream(_)), "got {error}");
        }
        drop(client);
        agent.finish().await;
    }
}

#[tokio::test]
async fn prompt_errors_show_the_reason_from_data() {
    let agent = FakeAgent::start(Scenario::PromptErrorData).await;
    let client = bounded(AcpClient::connect(&agent.socket, broker()), "initialize")
        .await
        .unwrap();
    let prompt_error = || {
        bounded(
            client.prompt("session-1", "hi", 1024, Duration::from_secs(2)),
            "prompt",
        )
    };
    // data.details 接在笼统的 message 后面，用户能看到要做什么。
    let shown = prompt_error().await.unwrap_err().to_string();
    assert!(
        shown.contains("Internal error")
            && shown.contains("No model selected. Use /login 选择模型。"),
        "用户看到的：{shown}"
    );
    // data 本身是字符串。
    let shown = prompt_error().await.unwrap_err().to_string();
    assert!(
        shown.contains("后台正在编译，请稍后再试"),
        "用户看到的：{shown}"
    );
    // detail 和 message 相同时不重复。
    assert_eq!(
        prompt_error().await.unwrap_err().to_string(),
        "ACP 上游出错：同一个原因"
    );
    // 其余形状照旧：不认识的 data 不往上接。
    assert_eq!(
        prompt_error().await.unwrap_err().to_string(),
        "ACP 上游出错：Internal error"
    );
    // 整段截到 2 KiB，不切开多字节字符。
    if let Error::Upstream(text) = prompt_error().await.unwrap_err() {
        assert!(
            text.len() > 2000 && text.len() <= 2048,
            "截断后 {} 字节",
            text.len()
        );
        assert!(text.starts_with("Internal error\n"));
    } else {
        panic!("期待 Upstream 错误");
    }
    drop(client);
    agent.finish().await;
}
