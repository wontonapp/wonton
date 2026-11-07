//! “我来操作”期间 Bot 的调用怎么走（ADR-0024）。

use std::time::Duration;

use rmcp::model::{CallToolRequestParams, CallToolResult};
use serde_json::{Value, json};
use wonton_protocol::Answer;

use super::tests::{Setup, setup, text};
use super::{Caller, PolicyChange};
use crate::plugin::PluginId;
use crate::plugin::fake_docker::FakeDocker;
use crate::plugin::languages::Languages;

const DELETE: &str = "app-example-notes__delete";
const SEARCH: &str = "app-example-notes__search";
const OWNER: &str = "owner_token";
const PHONE: &str = "device:phone";

async fn start(ttl: Duration) -> Setup {
    let (_docker, engine) = FakeDocker::start().await;
    setup(engine, ttl).await
}

fn notes() -> PluginId {
    PluginId::parse("app.example.notes").unwrap()
}

fn delete(s: &Setup, name: &str) -> tokio::task::JoinHandle<CallToolResult> {
    s.spawn(DELETE, json!({"name": name}))
}

fn posted(s: &Setup) -> usize {
    s.cards.posted.lock().unwrap().len()
}

fn deleted(s: &Setup) -> usize {
    s.notes.deleted.lock().unwrap().len()
}

fn takeovers_audit(s: &Setup) -> Vec<Value> {
    let lines = s.audit();
    lines
        .into_iter()
        .filter(|line| line.get("takeover").is_some())
        .collect()
}

fn failure(result: &CallToolResult) -> Option<&str> {
    result.meta.as_ref()?.0.get("app.wonton/failure")?.as_str()
}

/// 等一会，确认调用还停着：没发卡片，也没执行。
async fn assert_still_held(s: &Setup, call: &tokio::task::JoinHandle<CallToolResult>) {
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(!call.is_finished(), "调用应该还停着");
    assert_eq!(posted(s), 0, "停住时不发卡片");
    assert_eq!(deleted(s), 0, "停住时不执行");
}

#[tokio::test]
async fn a_held_call_asks_for_a_card_after_the_owner_hands_back() {
    let s = start(Duration::from_secs(5)).await;
    s.bus.renew_takeover(&notes(), PHONE);
    let call = delete(&s, "a");
    assert_still_held(&s, &call).await;

    s.bus.end_takeover(&notes(), PHONE);
    let card = s.cards.wait_for(1).await;
    assert_eq!(deleted(&s), 0, "交还后要主人确认才执行");
    s.answer(&card.id, Answer::Approve).unwrap();
    assert_eq!(text(&call.await.unwrap()), "删掉了 a.txt");

    let audit = s.audit();
    let line = audit.last().unwrap();
    assert_eq!(line["basis"], "card");
    assert!(line["held_ms"].as_u64().unwrap() >= 100, "{line}");
    let takeovers = takeovers_audit(&s);
    assert_eq!(takeovers[0]["takeover"], "started");
    assert_eq!(takeovers[0]["by"], PHONE);
    assert_eq!(takeovers[1]["takeover"], "ended");
    assert_eq!(takeovers[1]["reason"], "handed");
}

#[tokio::test]
async fn an_always_allowed_bot_runs_without_a_card_once_handed_back() {
    let s = start(Duration::from_secs(5)).await;
    let change = PolicyChange::Always {
        plugin: "app.example.notes",
        caller: "com.example.agent",
    };
    s.bus.change_tool_policy(change, true, OWNER).unwrap();
    s.bus.renew_takeover(&notes(), PHONE);
    let call = delete(&s, "a");
    assert_still_held(&s, &call).await;

    s.bus.end_takeover(&notes(), PHONE);
    assert_eq!(text(&call.await.unwrap()), "删掉了 a.txt");
    assert_eq!(posted(&s), 0);
    let line = s
        .audit()
        .into_iter()
        .rfind(|line| line.get("tool").is_some())
        .unwrap();
    assert_eq!(line["basis"], "always");
    assert!(line["held_ms"].is_u64());
}

#[tokio::test]
async fn read_only_tools_keep_working_during_a_takeover() {
    let s = start(Duration::from_secs(5)).await;
    s.bus.renew_takeover(&notes(), PHONE);
    let result = s.call(SEARCH, json!({"name": "a"})).await;
    assert_eq!(text(&result), "找到 a.txt");

    let change = PolicyChange::ReadOnly {
        plugin: "app.example.notes",
        tool: "delete",
    };
    s.bus.change_tool_policy(change, true, OWNER).unwrap();
    let result = s.call(DELETE, json!({"name": "b"})).await;
    assert_eq!(text(&result), "删掉了 b.txt", "主人标的只读也算");
    assert!(s.audit().last().unwrap().get("held_ms").is_none());
}

#[tokio::test]
async fn a_takeover_nobody_renews_ends_by_itself() {
    let s = start(Duration::from_secs(5)).await;
    s.bus.set_takeover_lease(Duration::from_millis(300));
    s.bus.renew_takeover(&notes(), PHONE);
    let call = delete(&s, "a");
    for _ in 0..4 {
        tokio::time::sleep(Duration::from_millis(150)).await;
        s.bus.renew_takeover(&notes(), PHONE);
    }
    assert_eq!(posted(&s), 0, "续着约就还在操作");

    let card = s.cards.wait_for(1).await;
    s.answer(&card.id, Answer::Reject).unwrap();
    call.await.unwrap();
    let takeovers = takeovers_audit(&s);
    assert_eq!(takeovers.len(), 2, "续约不写审计：{takeovers:?}");
    assert_eq!(takeovers[1]["reason"], "lease_lapsed");
}

#[tokio::test]
async fn it_ends_only_when_the_last_device_hands_back() {
    let s = start(Duration::from_secs(5)).await;
    s.bus.renew_takeover(&notes(), PHONE);
    s.bus.renew_takeover(&notes(), "device:tablet");
    let call = delete(&s, "a");
    s.bus.end_takeover(&notes(), PHONE);
    assert_still_held(&s, &call).await;

    s.bus.end_takeover(&notes(), PHONE);
    assert_still_held(&s, &call).await;

    s.bus.end_device_takeovers("tablet");
    let card = s.cards.wait_for(1).await;
    s.answer(&card.id, Answer::Reject).unwrap();
    call.await.unwrap();
    let takeovers = takeovers_audit(&s);
    assert_eq!(takeovers.len(), 2, "{takeovers:?}");
    assert_eq!(takeovers[1]["by"], "device:tablet");
    assert_eq!(takeovers[1]["reason"], "device_revoked");
}

#[tokio::test]
async fn a_plugin_that_stops_ends_the_takeover() {
    let s = start(Duration::from_secs(5)).await;
    s.bus.renew_takeover(&notes(), PHONE);
    let call = delete(&s, "a");
    assert_still_held(&s, &call).await;

    let manifest = crate::plugin::PluginPackage::parse(
        super::tests::NOTES,
        &Languages::from_header(Some("zh-Hans")),
    )
    .unwrap()
    .manifest;
    s.manager.register_for_test(
        manifest,
        crate::plugin::PluginStatus::Degraded {
            reason: "停了".to_owned(),
        },
        None,
    );
    s.manager
        .mcp_for_test(&notes(), "http://127.0.0.1:9/mcp".to_owned());

    tokio::time::timeout(Duration::from_secs(5), async {
        while takeovers_audit(&s).len() < 2 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(takeovers_audit(&s)[1]["reason"], "plugin_stopped");
    assert_eq!(deleted(&s), 0);
    call.abort();
}

#[tokio::test]
async fn a_call_whose_agent_leaves_during_the_hold_is_dropped() {
    let s = start(Duration::from_secs(5)).await;
    s.bus.renew_takeover(&notes(), PHONE);
    let (leave, left) = tokio::sync::oneshot::channel::<()>();
    let (bus, caller) = (s.bus.clone(), s.caller.clone());
    let call = tokio::spawn(async move {
        let request = CallToolRequestParams::new(DELETE)
            .with_arguments(json!({"name": "a"}).as_object().cloned().unwrap());
        let cancelled = async {
            let _ = left.await;
        };
        bus.call(&caller, request, cancelled).await
    });
    assert_still_held(&s, &call).await;

    leave.send(()).unwrap();
    assert_eq!(call.await.unwrap().is_error, Some(true));
    s.bus.end_takeover(&notes(), PHONE);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(posted(&s), 0, "主人没答应过，交还后也不发卡片");
    assert_eq!(deleted(&s), 0);
    let line = s
        .audit()
        .into_iter()
        .rfind(|line| line.get("tool").is_some())
        .unwrap();
    assert_eq!(line["decision"], "withdrawn");
}

#[tokio::test]
async fn holding_too_long_says_it_did_not_run() {
    let s = start(Duration::from_millis(200)).await;
    s.bus.renew_takeover(&notes(), PHONE);
    let result = s.call(DELETE, json!({"name": "a"})).await;
    assert_eq!(result.is_error, Some(true));
    assert_eq!(failure(&result), Some("owner_operating"));
    assert_eq!(deleted(&s), 0);
    assert_eq!(posted(&s), 0);
    assert_eq!(s.audit().last().unwrap()["outcome"], "owner_operating");
}

#[tokio::test]
async fn too_many_held_calls_are_refused_and_the_rest_go_to_the_card_limit() {
    let s = start(Duration::from_secs(5)).await;
    s.bus.renew_takeover(&notes(), PHONE);
    let held: Vec<_> = (0..16).map(|i| delete(&s, &format!("n{i}"))).collect();
    tokio::time::sleep(Duration::from_millis(200)).await;

    let result = s.call(DELETE, json!({"name": "over"})).await;
    assert_eq!(failure(&result), Some("owner_operating"));

    let (bus, other) = (
        s.bus.clone(),
        Caller::new(PluginId::parse("com.example.other").unwrap(), None),
    );
    let other = tokio::spawn(async move {
        let request = CallToolRequestParams::new(DELETE)
            .with_arguments(json!({"name": "z"}).as_object().cloned().unwrap());
        bus.call(&other, request, std::future::pending()).await
    });
    assert_still_held(&s, &other).await;

    // 交还后一起回到判定：每个调用方最多 3 张等待的卡片（ADR-0016），超过的直接拒绝。
    s.bus.end_takeover(&notes(), PHONE);
    let first = s.cards.wait_for(3).await;
    s.answer(&first.id, Answer::Reject).unwrap();
    let cards: Vec<_> = s.cards.posted.lock().unwrap().clone();
    for card in cards.iter().filter(|c| c.id != first.id) {
        let _ = s.answer(&card.id, Answer::Reject);
    }
    let mut refused = 0;
    for call in held {
        if call.await.unwrap().is_error == Some(true) {
            refused += 1;
        }
    }
    assert_eq!(refused, 16);
    assert_eq!(deleted(&s), 0);
    other.abort();
}
