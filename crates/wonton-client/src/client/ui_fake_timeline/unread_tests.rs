//! 未读数字与后台来消息。

use std::collections::HashSet;

use tokio::runtime::Handle;

use super::super::test_support::*;
use crate::chat::TimelineSubscription;

/// 翻走以后后台陆续来三条，数字跟着涨；回到底部清掉，再翻走重新从 1 数起。
#[tokio::test(start_paused = true)]
async fn arrivals_count_only_while_away() {
    let fakes = FakeTimelines::default();
    let subscription = fakes.subscribe(&target());
    let _ = next(&subscription).await;

    subscription.set_scroll(TimelineScroll::Away);
    for expected in 1..=3 {
        let update = next(&subscription).await;
        assert_eq!(
            update.viewport,
            TimelineViewport::Away {
                new_messages: expected
            }
        );
        assert!(!update.scroll_to_bottom, "来消息不该把壳拉到底");
    }

    subscription.set_scroll(TimelineScroll::Bottom);
    assert_eq!(next(&subscription).await.viewport, TimelineViewport::Bottom);

    subscription.set_scroll(TimelineScroll::Away);
    assert_eq!(
        next(&subscription).await.viewport,
        TimelineViewport::Away { new_messages: 1 }
    );
}

/// 停在别处以后打草稿、发消息那几批也带着当时的条数：照抄第一批的 0 会让按钮的数字先闪回 0。
#[tokio::test(start_paused = true)]
async fn away_count_survives_draft_and_send_batches() {
    let fakes = FakeTimelines::default();
    let subscription = fakes.subscribe(&target());
    let _ = next(&subscription).await;
    subscription.set_scroll(TimelineScroll::Away);
    let _ = next(&subscription).await;
    let _ = next(&subscription).await;

    // 后台可能又挤进来一条，所以只断“还是 Away 且数字没掉回 0”。
    let assert_kept = |update: &TimelineUpdate| {
        let TimelineViewport::Away { new_messages } = update.viewport else {
            panic!("停在别处却报贴底：{:?}", update.viewport);
        };
        assert!(new_messages > 0, "条数闪回了 0");
    };
    fakes.timeline_action(TimelineAction::SetDraft {
        target: target(),
        body: "草稿".into(),
        reply_to: None,
    });
    assert_kept(&next(&subscription).await);
    fakes.timeline_action(TimelineAction::SendText {
        target: target(),
        body: "发出的字".into(),
        reply_to: None,
        mentions: Vec::new(),
    });
    assert_kept(&next(&subscription).await);
}

/// 壳在 Swift 线程上报位置，那里没有 Tokio 上下文；后台的三条也得排得起来（排不起来会在壳里 panic）。
#[tokio::test(start_paused = true)]
async fn scroll_from_a_thread_without_runtime_still_starts_arrivals() {
    let fakes = FakeTimelines::new(Handle::current());
    let subscription = fakes.subscribe(&target());
    let _ = next(&subscription).await;

    let reported = subscription.clone();
    std::thread::spawn(move || reported.set_scroll(TimelineScroll::Away))
        .join()
        .unwrap();

    assert_eq!(
        next(&subscription).await.viewport,
        TimelineViewport::Away { new_messages: 1 }
    );
}

/// 壳里的 `reportPosition` 每动一下几何就往这报：从外面线程连报一串，订阅不能被自己报没了。
#[tokio::test]
async fn shell_position_reports_from_a_foreign_thread_keep_the_subscription_alive() {
    let dir = tempfile::tempdir().unwrap();
    let core =
        crate::client::ClientCore::new_ui_fake(dir.path().to_string_lossy().into_owned()).unwrap();
    let subscription = core.subscribe_timeline(target()).await.unwrap();
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), subscription.next()).await;

    let reported = subscription.clone();
    std::thread::spawn(move || {
        for _ in 0..40 {
            reported.set_scroll(TimelineScroll::Away);
            reported.set_scroll(TimelineScroll::Bottom);
        }
        reported.set_scroll(TimelineScroll::Away);
    })
    .join()
    .unwrap();

    // 连报一大串以后订阅还活着，最后那条「翻走」还能收到后台来的一条。
    let mut saw_bottom = false;
    let update = loop {
        let update = tokio::time::timeout(std::time::Duration::from_secs(2), subscription.next())
            .await
            .expect("订阅自己断了");
        let update = update.expect("订阅自己断了");
        match update.viewport {
            TimelineViewport::Away { new_messages } => break new_messages,
            TimelineViewport::Bottom => saw_bottom = true,
        }
    };
    assert!(saw_bottom, "中间的到底批次该到");
    assert_eq!(update, 1);
}

/// 翻走以后后台来三条：数字一条条涨，回到底部清掉，再翻走从 0 重数。走查时看未读数字就这么用。
#[tokio::test(start_paused = true)]
async fn staying_away_lets_three_messages_arrive() {
    let fakes = FakeTimelines::default();
    let subscription = fakes.subscribe(&target());
    let _first = next(&subscription).await;
    subscription.set_scroll(TimelineScroll::Away);
    let mut ids = Vec::new();
    for count in 1..=3 {
        let arrival = next(&subscription).await;
        assert_eq!(
            arrival.viewport,
            TimelineViewport::Away {
                new_messages: count
            }
        );
        let item = pushed(&arrival);
        let TimelineContent::Message { message } = &item.content else {
            panic!("{} 是消息", item.id);
        };
        assert!(!message.is_own);
        assert_eq!(message.status, None);
        ids.push(item.id.clone());
    }
    subscription.set_scroll(TimelineScroll::Bottom);
    wait_bottom(&subscription).await;
    subscription.set_scroll(TimelineScroll::Away);
    let again = next(&subscription).await;
    assert_eq!(again.viewport, TimelineViewport::Away { new_messages: 1 });
    ids.push(pushed(&again).id.clone());
    // 一轮没跑完就回到底部：剩下的条算在底部，数字清零；再翻走又从 1 重数。
    subscription.set_scroll(TimelineScroll::Bottom);
    wait_bottom(&subscription).await;
    wait_bottom(&subscription).await;
    subscription.set_scroll(TimelineScroll::Away);
    let third = next(&subscription).await;
    assert_eq!(third.viewport, TimelineViewport::Away { new_messages: 1 });
    ids.push(pushed(&third).id.clone());
    // 两轮下来 id 两两不同：壳按 id 当行键，重了 LazyColumn 会崩。
    let unique: HashSet<&String> = ids.iter().collect();
    assert_eq!(unique.len(), ids.len(), "id 撞了：{ids:?}");
}

/// 这一批里接在末尾的那条。
fn pushed(update: &TimelineUpdate) -> &TimelineItem {
    update
        .diffs
        .iter()
        .find_map(|diff| match diff {
            TimelineDiff::PushBack { item } => Some(item),
            _ => None,
        })
        .unwrap_or_else(|| panic!("新消息接在末尾：{:?}", update.diffs))
}

/// 回到底部让数字清零的那一批。
async fn wait_bottom(subscription: &TimelineSubscription) {
    loop {
        let update = next(subscription).await;
        if update.viewport == TimelineViewport::Bottom {
            return;
        }
    }
}
