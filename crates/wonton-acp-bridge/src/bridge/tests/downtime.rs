//! 停机积压：桥重启后，停机期间发来的消息要补处理；超出上限的如实列出，不假装处理过。

use std::time::Duration;

use wonton_matrix::MessageBody;

use super::*;
use crate::bridge::test_support::Effect;

#[tokio::test]
async fn messages_sent_while_down_are_caught_up() {
    let mut harness = Harness::start().await;
    harness.send_at("$m1", 100, "在线时说的话");
    let prompts = harness.wait_prompts(1).await;
    harness.respond(&prompts[0].0, "好").await;
    // 重启：快照里停机期间发的那条要补，重启前的不再重跑。
    harness
        .restart_with(vec![
            owner_message_at("$m1", "在线时说的话", None, 100),
            owner_message_at("$m2", "停机期间的话", None, 200),
        ])
        .await;
    let prompts = harness.wait_prompts(1).await;
    assert_eq!(prompts.len(), 1);
    assert_eq!(prompts[0].1, "停机期间的话");
    harness.finish().await;
}

#[tokio::test]
async fn the_first_start_does_not_replay_history() {
    let harness =
        Harness::start_with(vec![owner_message_at("$m0", "装插件之前的话", None, 500)]).await;
    // 首次运行没有已读位置：快照里的都是启动前的旧消息，不补也不出声。
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(harness.prompts().await.is_empty());
    // 快照整批记了已读位置：之后的实时消息照常处理。
    harness.send_at("$m1", 600, "启动后的话");
    let prompts = harness.wait_prompts(1).await;
    assert_eq!(prompts[0].1, "启动后的话");
    harness.finish().await;
}

#[tokio::test]
async fn backlog_beyond_the_cap_is_listed_not_processed() {
    let mut harness = Harness::start().await;
    harness.send_at("$m1", 100, "在线时说的话");
    let prompts = harness.wait_prompts(1).await;
    harness.respond(&prompts[0].0, "好").await;
    // 8 条停机消息：只补最近 6 条，前 2 条在房间里如实列出。
    let mut backlog = vec![owner_message_at("$m1", "在线时说的话", None, 100)];
    for index in 0..8 {
        backlog.push(owner_message_at(
            &format!("$d{index}"),
            &format!("停机第{index}句"),
            None,
            200 + index as u64,
        ));
    }
    harness.restart_with(backlog).await;
    for index in 2..8 {
        let prompts = harness.wait_prompts(1).await;
        assert_eq!(prompts[0].1, format!("停机第{index}句"), "补处理顺序错");
        harness.respond(&prompts[0].0, "好").await;
    }
    let notices_before = notices(&harness);
    assert_eq!(notices_before.len(), 1, "notices={notices_before:?}");
    assert!(
        notices_before[0].contains("2 条没有处理")
            && notices_before[0].contains("停机第0句")
            && notices_before[0].contains("停机第1句"),
        "notices={notices_before:?}"
    );
    // 再重启：丢下的已经在已读位置之后，新房间里不再重新列出、也不再补。
    harness.restart_with(Vec::new()).await;
    harness.send_at("$live", 400, "重启后的话");
    let prompts = harness.wait_prompts(1).await;
    assert_eq!(prompts[0].1, "重启后的话");
    assert!(
        notices(&harness).is_empty(),
        "notices={:?}",
        notices(&harness)
    );
    harness.finish().await;
}

#[tokio::test]
async fn messages_seen_before_the_restart_are_not_reprocessed_as_updates() {
    let mut harness = Harness::start().await;
    harness.send_at("$m1", 100, "重启前说过的话");
    let prompts = harness.wait_prompts(1).await;
    harness.respond(&prompts[0].0, "好").await;
    // 快照空（竞态）：旧消息稍后当增量又到一遍，不能重跑。
    harness.restart_with(Vec::new()).await;
    harness.send_at("$m1", 100, "重启前说过的话");
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(harness.prompts().await.is_empty());
    // 循环还活着：新的消息照常处理。
    harness.send_at("$m2", 300, "重启后的话");
    let prompts = harness.wait_prompts(1).await;
    assert_eq!(prompts[0].1, "重启后的话");
    harness.finish().await;
}

#[tokio::test]
async fn late_decryption_is_still_delivered_after_the_boundary_moved() {
    let harness = Harness::start().await;
    // 第一批没解开；第二批把已读位置推过它；第三批密钥到了、同一事件解开重投。
    let mut locked = owner_message_at("$m1", "迟解密的话", None, 100);
    locked.body = MessageBody::Undecryptable;
    harness.room.push(vec![locked]);
    harness.send_at("$m2", 200, "后来的话");
    let prompts = harness.wait_prompts(1).await;
    assert_eq!(prompts[0].1, "后来的话");
    harness.send_at("$m1", 100, "迟解密的话");
    // 上一轮还在跑：重投的这句按平时一样排队，等这轮结束才到 Agent。
    harness.respond(&prompts[0].0, "好").await;
    let prompts = harness.wait_prompts(1).await;
    assert_eq!(prompts[0].1, "迟解密的话");
    harness.finish().await;
}

fn notices(harness: &Harness) -> Vec<String> {
    harness
        .room
        .effects()
        .into_iter()
        .filter_map(|effect| match effect {
            Effect::Notice(text) => Some(text),
            _ => None,
        })
        .collect()
}
