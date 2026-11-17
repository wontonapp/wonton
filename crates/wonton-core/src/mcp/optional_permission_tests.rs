//! Bot 开关可选权限（ADR-0057）：敏感应用管理操作始终确认，不能被工具放宽绕过。

use serde_json::json;
use wonton_protocol::Answer;

use super::test_setup::setup_optional;
use super::tests::text;

const AGENT: &str = "com.example.agent";
const TOOL: &str = "wonton__set_plugin_permission";

fn notes_id() -> crate::plugin::PluginId {
    crate::plugin::PluginId::parse("app.example.notes").unwrap()
}

fn args(enabled: bool) -> serde_json::Value {
    json!({"id": "app.example.notes", "permission": "docker.socket", "enabled": enabled})
}

#[tokio::test]
async fn enabling_asks_every_time_even_with_every_relaxation_on() {
    let s = setup_optional().await;
    let policy = s.manager.tool_policy();
    policy.set_allow_all(AGENT, true).unwrap();

    for n in 1..=2 {
        if n == 2 {
            let off = s.spawn(TOOL, args(false));
            let card = s.cards.wait_for(2).await;
            s.answer(&card.id, Answer::Approve).unwrap();
            let off = off.await.unwrap();
            assert!(!off.is_error.unwrap_or(false), "{}", text(&off));
        }
        let call = s.spawn(TOOL, args(true));
        let card = s.cards.wait_for(n * 2 - 1).await;
        assert!(
            card.lines.iter().any(|line| line.text.contains("root")),
            "{:?}",
            card.lines
        );
        s.answer(&card.id, Answer::Approve).unwrap();
        let result = call.await.unwrap();
        assert!(!result.is_error.unwrap_or(false), "{}", text(&result));
        assert_eq!(s.audit().last().unwrap()["basis"], "card");
    }
    let record = s.manager.record_for_test(&notes_id()).unwrap();
    assert_eq!(record.enabled_permissions.len(), 1);
}

#[tokio::test]
async fn disabling_with_allow_all_still_asks() {
    let s = setup_optional().await;
    let turn_on = s.spawn(TOOL, args(true));
    let card = s.cards.wait_for(1).await;
    s.answer(&card.id, Answer::Approve).unwrap();
    turn_on.await.unwrap();

    s.manager.tool_policy().set_allow_all(AGENT, true).unwrap();
    let posted = s.cards.posted.lock().unwrap().len();
    let off = s.spawn(TOOL, args(false));
    let card = s.cards.wait_for(posted + 1).await;
    s.answer(&card.id, Answer::Approve).unwrap();
    let off = off.await.unwrap();
    assert!(!off.is_error.unwrap_or(false), "{}", text(&off));
    assert_eq!(s.cards.posted.lock().unwrap().len(), posted + 1);
    let record = s.manager.record_for_test(&notes_id()).unwrap();
    assert!(record.enabled_permissions.is_empty());
}

#[tokio::test]
async fn without_a_relaxation_disabling_asks() {
    let s = setup_optional().await;
    let turn_on = s.spawn(TOOL, args(true));
    let card = s.cards.wait_for(1).await;
    s.answer(&card.id, Answer::Approve).unwrap();
    turn_on.await.unwrap();

    let off = s.spawn(TOOL, args(false));
    let card = s.cards.wait_for(2).await;
    assert!(card.title.contains("关掉"), "{}", card.title);
    s.answer(&card.id, Answer::Approve).unwrap();
    off.await.unwrap();
    assert!(
        s.manager
            .record_for_test(&notes_id())
            .unwrap()
            .enabled_permissions
            .is_empty()
    );
}

#[tokio::test]
async fn a_permission_that_is_not_optional_is_refused() {
    let s = setup_optional().await;
    let result = s
        .call(
            TOOL,
            json!({"id": "app.example.notes", "permission": "network.host", "enabled": true}),
        )
        .await;
    assert_eq!(result.is_error, Some(true));
    assert_eq!(s.cards.posted.lock().unwrap().len(), 0);
}
