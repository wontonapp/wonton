//! 卡片上的放行时段（ADR-0024）。

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use rmcp::model::{CallToolRequestParams, CallToolResult};
use serde_json::{Value, json};
use wonton_protocol::{Answer, ConfirmationCard, ConfirmationState, WindowEnd};

use super::confirm::{AnswerError, Confirmations, DEFAULT_WINDOW};
use super::test_support::Cards;
use super::tests::{Setup, setup_window, text};
use super::{Caller, cards};
use crate::plugin::PluginId;

const DELETE: &str = "app-example-notes__delete";

fn delete_args(name: &str) -> Value {
    json!({"name": name})
}

fn other_bot(s: &Setup) -> tokio::task::JoinHandle<CallToolResult> {
    let bus = s.bus.clone();
    let other = Caller::new(PluginId::parse("com.example.other").unwrap(), None);
    tokio::spawn(async move {
        let request = CallToolRequestParams::new(DELETE)
            .with_arguments(delete_args("other").as_object().cloned().unwrap());
        bus.call(&other, request, std::future::pending()).await
    })
}

/// 卡片一出来就开的时段，和还没跑完的第一次调用。
struct Opened {
    card: ConfirmationCard,
    first: tokio::task::JoinHandle<CallToolResult>,
}

/// 发一次调用，点“都允许”，卡片一出来就返回（不等调用跑完，它的耗时不该算进时段）。
async fn open_window(s: &Setup) -> Opened {
    let first = s.spawn(DELETE, delete_args("first"));
    let n = s.cards.posted.lock().unwrap().len() + 1;
    let card = s.cards.wait_for(n).await;
    s.bus
        .answer_card(&card.id, Answer::AllowWindow, None, "owner_token")
        .unwrap();
    Opened { card, first }
}

fn posted(s: &Setup) -> usize {
    s.cards.posted.lock().unwrap().len()
}

fn last_update(s: &Setup, id: &str) -> ConfirmationCard {
    let updates = s.cards.updates.lock().unwrap();
    updates.iter().rev().find(|c| c.id == id).unwrap().clone()
}

fn windows_audit(s: &Setup) -> Vec<Value> {
    let lines = s.audit();
    lines
        .into_iter()
        .filter(|line| line.get("window").is_some())
        .collect()
}

/// 某次调用自己写的那条审计。时段打开和结束的行没有 `basis`，不能拿 `last()` 当这次调用的判定。
fn last_call(s: &Setup, name: &str) -> Value {
    s.audit()
        .into_iter()
        .rfind(|line| line.get("basis").is_some() && line["arguments"]["name"] == name)
        .unwrap()
}

fn ended(s: &Setup) -> bool {
    windows_audit(s)
        .iter()
        .any(|line| line["window"] == "ended")
}

#[tokio::test]
async fn a_window_clears_later_calls_and_calling_does_not_extend_it() {
    let s = setup_window(Duration::from_millis(600)).await;
    let Opened { card, first } = open_window(&s).await;
    let choices: Vec<_> = card.choices.iter().map(|c| c.answer).collect();
    assert_eq!(
        choices,
        [Answer::Approve, Answer::AllowWindow, Answer::Reject]
    );
    // 打开后立刻一起发：判定在每通调用进入时做，错开串行发只会让后面几通更贴近到期。
    let again: Vec<_> = (0..3)
        .map(|_| s.spawn(DELETE, delete_args("again")))
        .collect();
    for call in again {
        assert_eq!(text(&call.await.unwrap()), "删掉了 again.txt");
    }
    assert_eq!(text(&first.await.unwrap()), "删掉了 first.txt");
    assert_eq!(posted(&s), 1, "时段内不发卡片");
    // 等到期那条写进审计再读，复现它抢在调用后面。
    tokio::time::timeout(Duration::from_secs(2), async {
        while !ended(&s) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let line = last_call(&s, "again");
    assert_eq!(line["basis"], "window");
    assert_eq!(line["card"], card.id.as_str());

    let late = s.spawn(DELETE, delete_args("late"));
    let next = s.cards.wait_for(2).await;
    s.answer(&next.id, Answer::Reject).unwrap();
    late.await.unwrap();

    let ended = last_update(&s, &card.id).window.unwrap();
    assert_eq!(ended.ended, Some(WindowEnd::Expired));
    assert!(ended.allowed.unwrap() >= 2, "{ended:?}");
    let audit = windows_audit(&s);
    assert_eq!(audit[0]["window"], "opened");
    assert_eq!(audit[0]["by"], "owner_token");
    assert_eq!(audit[1]["window"], "ended");
    assert_eq!(audit[1]["reason"], "expired");
}

#[tokio::test]
async fn opening_releases_what_this_bot_is_waiting_on_but_not_other_bots() {
    let s = setup_window(DEFAULT_WINDOW).await;
    let waiting = s.spawn(DELETE, delete_args("waiting"));
    let waiting_card = s.cards.wait_for(1).await;
    let other = other_bot(&s);
    let other_card = s.cards.wait_for(2).await;

    let opened = open_window(&s).await;
    assert_eq!(text(&opened.first.await.unwrap()), "删掉了 first.txt");
    assert_eq!(text(&waiting.await.unwrap()), "删掉了 waiting.txt");
    let line = s
        .audit()
        .into_iter()
        .find(|line| line["arguments"]["name"] == "waiting")
        .unwrap();
    assert_eq!(line["basis"], "window");
    assert_eq!(line["card"], opened.card.id.as_str());
    assert!(matches!(
        s.cards.states(&waiting_card.id).last(),
        Some(ConfirmationState::Succeeded { .. })
    ));

    assert!(!other.is_finished(), "别的 Bot 的卡片还在等");
    s.answer(&other_card.id, Answer::Reject).unwrap();
    other.await.unwrap();

    let again = other_bot(&s);
    let card = s.cards.wait_for(4).await;
    s.answer(&card.id, Answer::Reject).unwrap();
    again.await.unwrap();
}

#[tokio::test]
async fn stopping_stops_later_calls_and_can_only_happen_once() {
    let s = setup_window(DEFAULT_WINDOW).await;
    let opened = open_window(&s).await;
    assert_eq!(text(&opened.first.await.unwrap()), "删掉了 first.txt");
    let card = opened.card;
    s.call(DELETE, delete_args("a")).await;

    let by = "device:abc";
    assert!(
        s.bus
            .end_window(&card.id, WindowEnd::Stopped, Some(by))
            .await
    );
    assert!(
        !s.bus
            .end_window(&card.id, WindowEnd::Stopped, Some(by))
            .await,
        "结束过的再停没有效果"
    );
    let window = last_update(&s, &card.id).window.unwrap();
    assert_eq!(window.ended, Some(WindowEnd::Stopped));
    assert_eq!(window.allowed, Some(1));
    let audit = windows_audit(&s);
    assert_eq!(audit.len(), 2);
    assert_eq!(audit[1]["by"], by);
    assert_eq!(audit[1]["allowed"], 1);

    let next = s.spawn(DELETE, delete_args("b"));
    let card = s.cards.wait_for(2).await;
    s.answer(&card.id, Answer::Reject).unwrap();
    next.await.unwrap();
}

#[tokio::test]
async fn core_tools_offer_no_window_and_refuse_the_answer() {
    let s = setup_window(DEFAULT_WINDOW).await;
    let install = s.spawn(
        "wonton__install_plugin",
        json!({"id": "app.example.journal"}),
    );
    let card = s.cards.wait_for(1).await;
    assert!(card.choices.iter().all(|c| c.answer != Answer::AllowWindow));
    assert_eq!(
        s.bus
            .answer_card(&card.id, Answer::AllowWindow, None, "owner_token"),
        Err(AnswerError::UnknownAnswer)
    );
    s.answer(&card.id, Answer::Reject).unwrap();
    install.await.unwrap();
}

/// 没有总线的场合：一张能开时段的卡片，点了“都允许”。
async fn open_on(confirmations: &Arc<Confirmations>, cards: &Arc<Cards>) -> String {
    let caller = PluginId::parse("com.example.agent").unwrap();
    let draft = cards::tool_call("Bot", "笔记", "delete", &json!({}), false)
        .with_window("app.example.notes", confirmations.window_length());
    let asking = confirmations.clone();
    let ask = tokio::spawn(async move { asking.ask(&caller, draft).await });
    let id = cards.wait_for(1).await.id;
    tokio::time::sleep(Duration::from_millis(50)).await;
    confirmations
        .answer(&id, Answer::AllowWindow, None)
        .unwrap();
    let ticket = ask.await.unwrap().unwrap();
    let done = ConfirmationState::Succeeded {
        message: "ok".to_owned(),
    };
    confirmations.finish(ticket, done).await;
    id
}

#[tokio::test]
async fn after_a_restart_the_window_is_gone_and_its_card_says_so_once_matrix_is_back() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("confirmation-windows.json");
    let cards = Arc::new(Cards::default());
    let ttl = Duration::from_secs(5);
    let first =
        Arc::new(Confirmations::new(cards.clone(), ttl).with_windows(DEFAULT_WINDOW, file.clone()));
    let id = open_on(&first, &cards).await;
    let caller = PluginId::parse("com.example.agent").unwrap();
    assert!(
        first
            .allow_in_window(&caller, "app.example.notes")
            .is_some()
    );

    cards.broken.store(true, Ordering::SeqCst);
    let restarted =
        Confirmations::new(cards.clone(), ttl).with_windows(DEFAULT_WINDOW, file.clone());
    assert_eq!(
        restarted.allow_in_window(&caller, "app.example.notes"),
        None
    );
    assert!(restarted.has_leftover_windows());
    assert!(restarted.recover_windows().await.is_empty());
    assert!(restarted.has_leftover_windows(), "没改成功的留着");

    cards.broken.store(false, Ordering::SeqCst);
    let ended = restarted.recover_windows().await;
    assert_eq!(ended[0].reason, WindowEnd::CoreRestarted);
    assert_eq!(ended[0].allowed, None);
    assert!(!restarted.has_leftover_windows());
    let updates = cards.updates.lock().unwrap();
    let last = updates.iter().rev().find(|card| card.id == id).unwrap();
    assert_eq!(
        last.window.as_ref().unwrap().ended,
        Some(WindowEnd::CoreRestarted)
    );
    drop(updates);
    let saved: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    assert_eq!(saved, json!({}));
}

#[tokio::test]
async fn a_window_opens_even_if_its_record_cannot_be_written() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("missing/confirmation-windows.json");
    let cards = Arc::new(Cards::default());
    let confirmations = Arc::new(
        Confirmations::new(cards.clone(), Duration::from_secs(5))
            .with_windows(DEFAULT_WINDOW, file),
    );
    open_on(&confirmations, &cards).await;
    let caller = PluginId::parse("com.example.agent").unwrap();
    assert!(
        confirmations
            .allow_in_window(&caller, "app.example.notes")
            .is_some()
    );
}

/// 放行时段只管 MCP 工具：同一个 Bot 等着的 Agent 权限卡片不跟着放行，那张卡片上也没有这个按钮（ADR-0035）。
#[tokio::test]
async fn a_window_does_not_answer_an_agents_own_permission_card() {
    let s = setup_window(DEFAULT_WINDOW).await;
    let (bus, caller) = (s.bus.clone(), s.caller.clone());
    let request = serde_json::from_value(json!({
        "tool_call_id": "call-1",
        "title": "运行 ls",
        "options": [{"choice_id": "once", "kind": "allow_once", "name": "这次允许"}],
    }))
    .unwrap();
    let asked = tokio::spawn(async move { bus.agent_permission(&caller, request).await });
    let agent_card = s.cards.wait_for(1).await;
    assert_eq!(
        s.bus
            .answer_card(&agent_card.id, Answer::AllowWindow, None, "owner_token"),
        Err(AnswerError::UnknownAnswer)
    );

    let opened = open_window(&s).await;
    assert_eq!(text(&opened.first.await.unwrap()), "删掉了 first.txt");
    assert!(!asked.is_finished());
    let waiting = s.bus.confirmations().pending();
    assert_eq!(waiting.len(), 1);
    assert_eq!(waiting[0].id, agent_card.id);
    asked.abort();
}
