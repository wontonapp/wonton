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
