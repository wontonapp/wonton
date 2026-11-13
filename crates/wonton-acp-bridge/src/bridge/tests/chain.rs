//! 轮次登记和停链（ADR-0061 第 10 节）：每轮先向核心领链再发提示词，被拒或核心
//! 不可用时不发并如实说明；出箱的 `CancelTurn` 取消会话并回报。

use std::time::Duration;

use super::*;
use crate::bridge::test_support::{Effect, wait_for, wait_until};
use wonton_protocol::{ChainError, ChainLimitKind, OutboxItem, OutboxKind};

/// 主对话的一轮：先给核心报 Start（房间、说话人、事件 ID、会话），再发提示词，
/// 结束后报 End。
#[tokio::test]
async fn a_turn_reports_start_prompts_then_end() {
    let harness = Harness::start().await;
    harness.send("$e1", "改登录");
    let prompts = harness.wait_prompts(1).await;
    let session = prompts[0].0.clone();

    let reports = harness.core.chain_reports();
    let start = &reports[0];
    assert_eq!(start["phase"], "start");
    assert_eq!(start["room"], "!room:example.org");
    assert_eq!(start["sender"], "@owner:example.org");
    assert_eq!(start["event_id"], "$e1");
    assert_eq!(start["session"], session);
    assert!(start.get("thread").is_none(), "主对话没有话题");

    harness.respond(&session, "登录改好了").await;
    wait_for(|| {
        let core = &harness.core;
        let session = session.clone();
        async move {
            core.chain_reports()
                .iter()
                .any(|report| report["phase"] == "end" && report["session"] == session.as_str())
        }
    })
    .await;
    harness.finish().await;
}

/// 话题里的一轮报的 `thread` 是话题根。
#[tokio::test]
async fn a_topic_turn_reports_the_thread_root() {
    let harness = Harness::start().await;
    harness.send_in_thread("$e1", "$root-1", "整理照片");
    let prompts = harness.wait_prompts(1).await;
    let session = prompts[0].0.clone();

    let reports = harness.core.chain_reports();
    assert_eq!(reports[0]["phase"], "start");
    assert_eq!(reports[0]["thread"], "$root-1");
    assert_eq!(reports[0]["session"], session);

    harness.respond(&session, "好了").await;
    wait_for(|| {
        let core = &harness.core;
        async move {
            core.chain_reports()
                .iter()
                .any(|report| report["phase"] == "end")
        }
    })
    .await;
    harness.finish().await;
}

/// 轮数到上限：假 Agent 什么都没收到，房间里一条 m.notice 说明是轮数。
#[tokio::test]
async fn a_round_limit_start_sends_a_notice_and_never_prompts() {
    let harness = Harness::start().await;
    harness.core.push_chain_reply(
        429,
        serde_json::to_value(ChainError::LimitReached {
            kind: ChainLimitKind::Rounds,
        })
        .unwrap(),
    );
    harness.send("$e1", "改登录");

    wait_until(|| {
        harness
            .room
            .effects()
            .iter()
            .any(|effect| matches!(effect, Effect::Notice(text) if text.contains("轮数")))
    })
    .await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        harness.prompts().await.is_empty(),
        "被拒的一轮不能发给 Agent"
    );
    assert!(
        !harness.room.receipts().contains(&"$e1".to_owned()),
        "Agent 没收到，不发已读回执"
    );
    harness.finish().await;
}

/// 链被停：一条 m.notice 说明已停。
#[tokio::test]
async fn a_stopped_start_says_so_in_a_notice() {
    let harness = Harness::start().await;
    harness
        .core
        .push_chain_reply(409, serde_json::to_value(ChainError::Stopped).unwrap());
    harness.send("$e1", "改登录");

    wait_until(|| {
        harness
            .room
            .effects()
            .iter()
            .any(|effect| matches!(effect, Effect::Notice(text) if text.contains("停")))
    })
    .await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(harness.prompts().await.is_empty());
    harness.finish().await;
}

/// 核心不可用：不发给 Agent，按类别报错（普通错误文字，不是 notice）。
#[tokio::test]
async fn an_unreachable_core_reports_an_error_and_never_prompts() {
    let harness = Harness::start().await;
    harness
        .core
        .push_chain_reply(500, serde_json::json!({"error": "storage"}));
    harness.send("$e1", "改登录");

    wait_until(|| {
        harness
            .room
            .effects()
            .iter()
            .any(|effect| matches!(effect, Effect::MainText(text) if text.contains("传输不可用")))
    })
    .await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(harness.prompts().await.is_empty());
    harness.finish().await;
}

/// 出箱来 `CancelTurn`：Agent 收到 `session/cancel`，未决的权限请求回 `cancelled`、
/// 挂在核心上的长轮询断开，End 报上，出箱回报发成。
#[tokio::test]
async fn a_cancel_turn_cancels_the_running_turn_and_its_permission() {
    let harness = Harness::start().await;
    harness.send("$e1", "改登录");
    let prompts = harness.wait_prompts(1).await;
    let session = prompts[0].0.clone();

    let id = harness.agent.ask_permission(&session, "src/a.rs").await;
    wait_until(|| harness.core.pending() == 1).await;

    harness.core.feed(OutboxItem {
        id: "o-1".into(),
        kind: OutboxKind::CancelTurn {
            chain: "chain-1".into(),
            session: session.clone(),
        },
    });

    // Agent 收到 session/cancel。
    wait_for(|| {
        let agent = &harness.agent;
        let session = session.clone();
        async move {
            agent.incoming().await.iter().any(|message| {
                message["method"] == "session/cancel"
                    && message["params"]["sessionId"] == session.as_str()
            })
        }
    })
    .await;
    // 权限请求回 cancelled，挂着的长轮询断开。
    wait_until(|| harness.core.dropped_permissions() == 1).await;
    let responses = harness.agent.responses_for(&id).await;
    assert_eq!(responses[0]["result"]["outcome"]["outcome"], "cancelled");
    // 这一轮以 cancelled 结束，End 报上；出箱回报发成。
    wait_for(|| {
        let core = &harness.core;
        let session = session.clone();
        async move {
            core.chain_reports()
                .iter()
                .any(|report| report["phase"] == "end" && report["session"] == session.as_str())
        }
    })
    .await;
    wait_for(|| {
        let core = &harness.core;
        async move {
            core.receipts()
                .iter()
                .any(|receipt| receipt["id"] == "o-1" && receipt["result"]["result"] == "sent")
        }
    })
    .await;
    harness.finish().await;
}

/// `CancelTurn` 指向不存在的会话：回报失败，在跑的会话不受影响。
#[tokio::test]
async fn a_cancel_turn_for_an_unknown_session_reports_failure() {
    let harness = Harness::start().await;
    harness.send("$e1", "改登录");
    let prompts = harness.wait_prompts(1).await;
    let session = prompts[0].0.clone();

    harness.core.feed(OutboxItem {
        id: "o-1".into(),
        kind: OutboxKind::CancelTurn {
            chain: "chain-1".into(),
            session: "ghost".into(),
        },
    });

    wait_for(|| {
        let core = &harness.core;
        async move {
            core.receipts()
                .iter()
                .any(|receipt| receipt["id"] == "o-1" && receipt["result"]["result"] == "failed")
        }
    })
    .await;
    assert!(
        harness
            .prompts()
            .await
            .iter()
            .any(|(pending, _)| *pending == session),
        "别的会话不受影响"
    );
    harness.finish().await;
}

/// 发消息的项真的去投递（测试的私聊没有房间，回报 `no_room`）；不认识的项照实回报
/// 失败并跳过，不猜怎么执行。
#[tokio::test]
async fn message_items_reach_the_dm_and_unknown_ones_are_skipped() {
    let harness = Harness::start().await;
    harness.core.feed(OutboxItem {
        id: "o-m".into(),
        kind: OutboxKind::Message {
            recipient: "mom".into(),
            body: "周末回来吗".into(),
        },
    });
    harness
        .core
        .feed_raw(serde_json::json!({"id": "o-x", "kind": {"type": "wat"}}));

    wait_for(|| {
        let core = &harness.core;
        async move {
            let receipts = core.receipts();
            let of = |id: &str| {
                receipts
                    .iter()
                    .find(|receipt| receipt["id"] == id)
                    .map(|receipt| receipt["result"].clone())
            };
            receipts.len() == 2
                && of("o-m") == Some(serde_json::json!({"result": "failed", "category": "no_room"}))
                && of("o-x") == Some(serde_json::json!({"result": "failed", "category": "other"}))
        }
    })
    .await;
    harness.finish().await;
}

/// 同一会话的下一句话报 Start 是另一轮（链的登记归核心，桥每轮都报）。
#[tokio::test]
async fn every_turn_reports_a_start() {
    let harness = Harness::start().await;
    harness.send("$e1", "第一句");
    let prompts = harness.wait_prompts(1).await;
    let session = prompts[0].0.clone();
    harness.respond(&session, "好").await;
    wait_for(|| {
        let core = &harness.core;
        async move {
            core.chain_reports()
                .iter()
                .any(|report| report["phase"] == "end")
        }
    })
    .await;

    harness.send("$e2", "第二句");
    wait_for(|| {
        let core = &harness.core;
        let session = session.clone();
        async move {
            core.chain_reports().iter().any(|report| {
                report["phase"] == "start"
                    && report["event_id"] == "$e2"
                    && report["session"] == session.as_str()
            })
        }
    })
    .await;
    harness.finish().await;
}

/// 核心每轮重新领的链传给真正的 ACP 请求，复用会话不复用旧链。
#[tokio::test]
async fn every_prompt_carries_its_own_grant_on_the_existing_session() {
    use serde_json::json;
    use wonton_protocol::ACP_CHAIN_META_KEY;

    let harness = Harness::start().await;
    harness
        .core
        .push_chain_reply(200, json!({"chain": "first-grant", "layer": 3}));
    harness.send("$e1", "第一句");
    let prompts = harness.wait_prompts(1).await;
    let session = prompts[0].0.clone();
    let requests = harness.agent.requests().await;
    let first = requests
        .iter()
        .find(|request| request["method"] == "session/prompt")
        .unwrap();
    assert_eq!(
        first["params"]["_meta"][ACP_CHAIN_META_KEY],
        json!({"id": "first-grant", "depth": 3})
    );
    let created = requests
        .iter()
        .find(|request| request["method"] == "session/new")
        .unwrap();
    assert!(created["params"]["_meta"].get(ACP_CHAIN_META_KEY).is_none());

    harness.respond(&session, "好").await;
    wait_for(|| {
        let core = &harness.core;
        async move {
            core.chain_reports()
                .iter()
                .any(|report| report["phase"] == "end")
        }
    })
    .await;
    // End 已到达核心，下一条排好的应答只会被第二次 Start 取走。
    harness
        .core
        .push_chain_reply(200, json!({"chain": "next-grant", "layer": 7}));
    harness.send("$e2", "第二句");
    let prompts = harness.wait_prompts(1).await;
    assert_eq!(prompts[0].0, session);
    let requests = harness.agent.requests().await;
    let turns: Vec<_> = requests
        .iter()
        .filter(|request| request["method"] == "session/prompt")
        .collect();
    assert_eq!(turns.len(), 2);
    assert_eq!(turns[1]["params"]["sessionId"], session);
    assert_eq!(
        turns[1]["params"]["_meta"][ACP_CHAIN_META_KEY],
        json!({"id": "next-grant", "depth": 7})
    );
    assert_eq!(
        requests
            .iter()
            .filter(|request| request["method"] == "session/new")
            .count(),
        1
    );
    harness.finish().await;
}
