//! Agent 权限确认的行为（ADR-0035）：卡片怎么显示，主人的回答怎么变成结果，审计写什么。

use std::sync::atomic::Ordering;
use std::time::Duration;

use serde_json::{Value, json};
use wonton_protocol::{
    AgentPermissionOutcome, AgentPermissionRequest, Answer, ConfirmationCard, ConfirmationState,
};

use super::AgentPermissionError;
use super::tests::{Setup, setup};
use crate::plugin::docker::Engine;

fn request(options: Value) -> AgentPermissionRequest {
    serde_json::from_value(json!({
        "tool_call_id": "call-1",
        "title": "运行 ls",
        "description": "列出工作区的文件",
        "arguments": {"command": "ls"},
        "options": options,
    }))
    .unwrap()
}

fn full_options() -> Value {
    json!([
        {"choice_id": "once", "kind": "allow_once", "name": "这次允许"},
        {"choice_id": "always", "kind": "allow_always", "name": "总是允许"},
        {"choice_id": "no", "kind": "reject_once", "name": "拒绝"},
    ])
}

async fn asking(
    s: &Setup,
    options: Value,
) -> (
    ConfirmationCard,
    tokio::task::JoinHandle<Result<AgentPermissionOutcome, AgentPermissionError>>,
) {
    let (bus, caller) = (s.bus.clone(), s.caller.clone());
    let request = request(options);
    let asked = tokio::spawn(async move { bus.agent_permission(&caller, request).await });
    (s.cards.wait_for(1).await, asked)
}

async fn setup_with_ttl(ttl: Duration) -> Setup {
    setup(Engine::unreachable_for_test(), ttl).await
}

#[tokio::test]
async fn the_owner_picks_one_of_the_agents_options() {
    let s = setup_with_ttl(Duration::from_secs(60)).await;
    let (card, asked) = asking(&s, full_options()).await;
    assert!(card.title.contains("运行 ls"), "{}", card.title);
    assert!(card.title.contains(&s.caller.name), "{}", card.title);
    let lines: Vec<_> = card.lines.iter().map(|l| l.text.as_str()).collect();
    assert_eq!(lines[0], "列出工作区的文件");
    assert!(lines[1].contains("ls"), "{lines:?}");
    let answers: Vec<_> = card.choices.iter().map(|c| c.answer).collect();
    assert_eq!(answers, [Answer::Approve, Answer::Approve, Answer::Reject]);
    let labels: Vec<_> = card.choices.iter().map(|c| c.label.as_str()).collect();
    assert_eq!(labels, ["这次允许", "总是允许", "拒绝"]);

    s.bus
        .confirmations()
        .answer(&card.id, Answer::Approve, Some("always"))
        .unwrap();
    assert_eq!(
        asked.await.unwrap(),
        Ok(AgentPermissionOutcome::Selected {
            choice_id: "always".to_owned()
        })
    );
    assert!(
        matches!(
            s.cards.states(&card.id).last(),
            Some(ConfirmationState::Succeeded { message }) if message.contains("总是允许")
        ),
        "卡片写明选了哪一项"
    );
    let audit = s.audit();
    let line = audit.last().unwrap();
    assert_eq!(line["tool_call_id"], "call-1");
    assert_eq!(line["decision"], "approved");
    assert_eq!(line["choice_id"], "always");
    assert_eq!(line["kind"], "allow_always");
}

/// `allow_always` 是 Agent 自己的作用域：不写工具策略，核心这边下一次还是要问。
#[tokio::test]
async fn allow_always_does_not_change_the_tool_policy() {
    let s = setup_with_ttl(Duration::from_secs(60)).await;
    let (card, asked) = asking(&s, full_options()).await;
    s.bus
        .confirmations()
        .answer(&card.id, Answer::Approve, Some("always"))
        .unwrap();
    asked.await.unwrap().unwrap();

    let delete = s.spawn("app-example-notes__delete", json!({"name": "b"}));
    let tool_card = s.cards.wait_for(2).await;
    assert_ne!(tool_card.id, card.id);
    s.answer(&tool_card.id, Answer::Reject).unwrap();
    delete.await.unwrap();
}

#[tokio::test]
async fn a_rejecting_option_comes_back_as_selected() {
    let s = setup_with_ttl(Duration::from_secs(60)).await;
    let (card, asked) = asking(&s, full_options()).await;
    s.bus
        .confirmations()
        .answer(&card.id, Answer::Reject, Some("no"))
        .unwrap();
    assert_eq!(
        asked.await.unwrap(),
        Ok(AgentPermissionOutcome::Selected {
            choice_id: "no".to_owned()
        })
    );
    assert_eq!(
        s.cards.states(&card.id).last(),
        Some(&ConfirmationState::Rejected)
    );
    let audit = s.audit();
    assert_eq!(audit.last().unwrap()["decision"], "rejected");
    assert_eq!(audit.last().unwrap()["kind"], "reject_once");
}

#[tokio::test]
async fn without_a_rejecting_option_the_card_offers_cancel() {
    let s = setup_with_ttl(Duration::from_secs(60)).await;
    let options = json!([{"choice_id": "once", "kind": "allow_once", "name": "这次允许"}]);
    let (card, asked) = asking(&s, options).await;
    let cancel = card.choices.last().unwrap();
    assert_eq!(cancel.answer, Answer::Reject);
    assert_eq!(cancel.label, "取消");
    assert_eq!(cancel.choice_id, None);

    s.bus
        .confirmations()
        .answer(&card.id, Answer::Reject, None)
        .unwrap();
    assert_eq!(asked.await.unwrap(), Ok(AgentPermissionOutcome::Cancelled));
    assert_eq!(
        s.cards.states(&card.id).last(),
        Some(&ConfirmationState::Rejected)
    );
}

#[tokio::test]
async fn an_unanswered_card_expires() {
    let s = setup_with_ttl(Duration::from_millis(200)).await;
    let (card, asked) = asking(&s, full_options()).await;
    assert_eq!(asked.await.unwrap(), Ok(AgentPermissionOutcome::Expired));
    assert_eq!(
        s.cards.states(&card.id).last(),
        Some(&ConfirmationState::Expired)
    );
    assert_eq!(s.audit().last().unwrap()["decision"], "expired");
}

#[tokio::test]
async fn bad_requests_never_reach_the_owner() {
    let s = setup_with_ttl(Duration::from_secs(60)).await;
    let twice = json!([
        {"choice_id": "a", "kind": "allow_once", "name": "这次允许"},
        {"choice_id": "a", "kind": "reject_once", "name": "拒绝"},
    ]);
    let blank = json!([{"choice_id": "", "kind": "allow_once", "name": "这次允许"}]);
    for options in [json!([]), twice, blank] {
        let result = s.bus.agent_permission(&s.caller, request(options)).await;
        assert!(
            matches!(result, Err(AgentPermissionError::InvalidInput(_))),
            "{result:?}"
        );
    }
    assert!(s.cards.posted.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_card_that_cannot_be_sent_is_reported() {
    let s = setup_with_ttl(Duration::from_secs(60)).await;
    s.cards.broken.store(true, Ordering::SeqCst);
    let result = s
        .bus
        .agent_permission(&s.caller, request(full_options()))
        .await;
    assert!(
        matches!(result, Err(AgentPermissionError::Unavailable(_))),
        "{result:?}"
    );
    assert_eq!(s.audit().last().unwrap()["decision"], "unconfirmed");
}

#[tokio::test]
async fn only_so_many_requests_wait_at_once() {
    let s = setup_with_ttl(Duration::from_secs(60)).await;
    let mut waiting = Vec::new();
    for _ in 0..3 {
        let (bus, caller) = (s.bus.clone(), s.caller.clone());
        let request = request(full_options());
        waiting.push(tokio::spawn(async move {
            bus.agent_permission(&caller, request).await
        }));
    }
    s.cards.wait_for(3).await;
    let result = s
        .bus
        .agent_permission(&s.caller, request(full_options()))
        .await;
    assert!(
        matches!(result, Err(AgentPermissionError::TooManyPending(_))),
        "{result:?}"
    );
    waiting.iter().for_each(tokio::task::JoinHandle::abort);
}

/// 调用方断开：卡片作废，主人再答得到“回答过了”。
#[tokio::test]
async fn a_caller_that_goes_away_withdraws_the_card() {
    let s = setup_with_ttl(Duration::from_secs(60)).await;
    let (card, asked) = asking(&s, full_options()).await;
    asked.abort();
    let _ = asked.await;
    for _ in 0..100 {
        if !s.cards.states(&card.id).is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(
        matches!(
            s.cards.states(&card.id).last(),
            Some(ConfirmationState::Failed { message }) if message.contains("作废")
        ),
        "{:?}",
        s.cards.states(&card.id)
    );
    assert!(s.bus.confirmations().pending().is_empty());
    assert_eq!(s.audit().last().unwrap()["decision"], "withdrawn");
}
