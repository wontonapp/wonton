//! 订阅的建立与结束。

use std::time::Duration;

use super::super::test_support::*;
use super::*;

/// 同一个房间能同时订两份（主对话和盖在上面的旁聊）：两份都拿到整条时间线，发出去的字两份都收得到，
/// 谁先订着谁也不会被挤掉。
#[tokio::test]
async fn two_subscriptions_share_one_room() {
    let fakes = FakeTimelines::default();
    let first = fakes.subscribe(&target());
    let _ = next(&first).await;
    let second = fakes.subscribe(&target());
    let batch = next(&second).await;
    let [TimelineDiff::Reset { items }] = batch.diffs.as_slice() else {
        panic!("后订的那份也拿到整条时间线");
    };
    assert_eq!(items.len(), ITEMS + 3);

    fakes.timeline_action(TimelineAction::SendText {
        target: target(),
        body: "Fixture both".into(),
        reply_to: None,
        mentions: Vec::new(),
    });
    assert!(!next(&second).await.diffs.is_empty());
    assert!(!next(&first).await.diffs.is_empty());
    // 后订的那份订阅时会给先订的那份也推一批，这里已经收过；两边的通道都没断。
}

/// 自己结束：发送端丢掉，壳的 `next()` 回 `None`；同一个目标再订时是一份新的。
#[tokio::test]
async fn end_closes_the_subscription() {
    let fakes = FakeTimelines::default();
    let subscription = fakes.subscribe(&target());
    let _ = next(&subscription).await;

    assert!(fakes.end(&target()));
    assert!(subscription.next().await.is_none());
    assert!(!fakes.end(&target()), "已经结束的不再算订着");
    let renewed = fakes.subscribe(&target());
    assert_eq!(next(&renewed).await.diffs.len(), 1, "重新订到的是新的一份");
}

/// 壳订阅走的是 Swift 线程，那里没有 Tokio 上下文：起后台那几条用的必须是核心的运行时而非当前上下文。
#[test]
fn subscribe_from_a_thread_without_runtime_still_fills_the_first_batch() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let fakes = Arc::new(FakeTimelines::new(runtime.handle().clone()));

    let subscribing = fakes.clone();
    let subscription = std::thread::spawn(move || subscribing.subscribe(&target()))
        .join()
        .unwrap();

    runtime.block_on(async {
        let first = tokio::time::timeout(Duration::from_secs(2), subscription.next())
            .await
            .expect("第一批该马上到")
            .expect("假房间一直开着");
        assert!(!first.diffs.is_empty());
    });
}

/// 走壳用的入口订阅一条时间线：第一批之后不该自己结束（壳那边一结束就当房间没了、把页收起来）。
#[tokio::test]
async fn a_subscription_does_not_end_by_itself() {
    let dir = tempfile::tempdir().unwrap();
    let core =
        crate::client::ClientCore::new_ui_fake(dir.path().to_string_lossy().into_owned()).unwrap();
    let subscription = core.subscribe_timeline(target()).await.unwrap();
    let first = tokio::time::timeout(std::time::Duration::from_secs(2), subscription.next()).await;
    assert!(matches!(first, Ok(Some(_))), "第一批该马上到");
    core.dispatch(wonton_protocol::Action::Timeline {
        action: TimelineAction::SendText {
            target: target(),
            body: "Fixture alive".into(),
            reply_to: None,
            mentions: Vec::new(),
        },
    })
    .unwrap();
    let sent = tokio::time::timeout(std::time::Duration::from_secs(2), subscription.next()).await;
    assert!(matches!(sent, Ok(Some(_))), "发出去的字该推过来");
    // 没事发生的时候也不该结束：结束的话壳会把这条时间线当成没了。
    let idle =
        tokio::time::timeout(std::time::Duration::from_millis(300), subscription.next()).await;
    assert!(idle.is_err(), "没有操作时订阅自己结束了");
}

/// 旁聊的批次多一张旁聊卡：旁聊页顶栏下那行要有东西可画。
#[tokio::test]
async fn a_side_chat_batch_carries_the_header_card() {
    let fakes = FakeTimelines::default();
    let root = crate::client::ui_fake_room::side_card().root_event_id;
    let subscription = fakes.subscribe(&TimelineTarget {
        room_id: ROOM.into(),
        thread_root: Some(root.clone()),
    });
    let first = next(&subscription).await;
    assert_eq!(first.side_chat.map(|card| card.root_event_id), Some(root));
}
