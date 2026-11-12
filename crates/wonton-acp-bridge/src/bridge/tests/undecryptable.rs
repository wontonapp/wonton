//! 解不开的加密消息：等过时限还没解开就说一次，密钥晚到解开的不说，名单外的人不说，不刷屏。

use std::time::Duration;

use super::*;
use wonton_matrix::{Message, MessageBody, SendStatus};

fn message_from(sender: &str, event_id: &str, undecryptable: bool) -> Message {
    Message {
        event_id: Some(event_id.parse().unwrap()),
        sender: sender.parse().unwrap(),
        sender_name: String::new(),
        is_own: false,
        timestamp_ms: 1,
        status: SendStatus::Sent,
        body: if undecryptable {
            MessageBody::Undecryptable
        } else {
            MessageBody::Text("解开了".into())
        },
        custom: Default::default(),
        mentions: Vec::new(),
        thread_root: None,
        edited: false,
        read_by: Vec::new(),
        reply: None,
        agent_run: None,
        group: None,
        can_redact: false,
    }
}

#[tokio::test]
async fn a_stuck_undecryptable_message_triggers_one_notice() {
    let harness = Harness::with(|config| {
        config.limits.undecryptable_after = Duration::from_millis(300);
        config.limits.undecryptable_every = Duration::from_secs(600);
    })
    .await;
    harness.room.push(vec![message_from(OWNER, "$u1", true)]);
    wait_until(|| {
        harness
            .room
            .main_texts()
            .iter()
            .any(|text| text.contains("读不了"))
    })
    .await;
    // 间隔内不再说：又来一条解不开的也只还是那一句。
    harness.room.push(vec![message_from(OWNER, "$u2", true)]);
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert_eq!(harness.room.main_texts().len(), 1);
    crate::health::clear_state_for_test();
    harness.finish().await;
}

#[tokio::test]
async fn a_message_that_decrypts_in_time_is_never_reported() {
    let harness = Harness::with(|config| {
        config.limits.undecryptable_after = Duration::from_millis(300);
        config.limits.undecryptable_every = Duration::from_secs(600);
    })
    .await;
    // 密钥晚到：同一条事件先解不开、马上解开（桥的流里正文更新成文字）。
    harness.room.push(vec![message_from(OWNER, "$u1", true)]);
    harness.room.push(vec![message_from(OWNER, "$u1", false)]);
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert!(harness.room.main_texts().is_empty());
    harness.finish().await;
}

#[tokio::test]
async fn the_notice_repeats_only_after_the_interval() {
    let harness = Harness::with(|config| {
        config.limits.undecryptable_after = Duration::from_millis(200);
        config.limits.undecryptable_every = Duration::from_millis(500);
    })
    .await;
    harness.room.push(vec![message_from(OWNER, "$u1", true)]);
    wait_until(|| harness.room.main_texts().len() == 1).await;
    // 事件一直没解开：过了间隔再提醒一次，而不是永远沉默或每轮都说。
    wait_until(|| harness.room.main_texts().len() == 2).await;
    crate::health::clear_state_for_test();
    harness.finish().await;
}

#[tokio::test]
async fn strangers_undecryptable_messages_are_ignored() {
    let harness = Harness::with(|config| {
        config.limits.undecryptable_after = Duration::from_millis(300);
        config.limits.undecryptable_every = Duration::from_secs(600);
    })
    .await;
    harness
        .room
        .push(vec![message_from("@stranger:example.org", "$u1", true)]);
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert!(harness.room.main_texts().is_empty());
    harness.finish().await;
}
