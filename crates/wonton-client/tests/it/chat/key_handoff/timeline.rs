//! 新手机没拿到密钥、房间里全是解不开的历史：时间线要安静下来，新消息照样送到，解不开的照实显示。

use std::sync::Arc;

use wonton_client::{MessageContent, TimelineContent};
use wonton_matrix::{ChatRoom, OwnedRoomId};
use wonton_protocol::TimelineTarget;

use super::super::accept_the_bots_encrypted_invite;
use super::super::support::{Home, Phone, Timeline, WAIT};
use super::{HISTORY, invite, message_text, old_phone, paginate_backwards, pair};

/// 一批增量里带的条目 ID：新消息到了没有数它。
fn diff_items(diff: &wonton_client::TimelineDiff) -> &[wonton_client::TimelineItem] {
    match diff {
        wonton_client::TimelineDiff::Append { items }
        | wonton_client::TimelineDiff::Reset { items } => items,
        wonton_client::TimelineDiff::PushFront { item }
        | wonton_client::TimelineDiff::PushBack { item }
        | wonton_client::TimelineDiff::Insert { item, .. }
        | wonton_client::TimelineDiff::Set { item, .. } => std::slice::from_ref(item),
        _ => &[],
    }
}

/// 排到订阅安静下来：300 ms 没有新批次算安静，最多等 3 秒。返回用了多久、共收到几批、有没有等到安静。
/// 订阅空转时一秒几千批，永远等不到这 300 ms。
async fn until_quiet(
    subscription: &wonton_client::TimelineSubscription,
) -> (std::time::Duration, usize, bool) {
    use std::time::{Duration, Instant};
    let started = Instant::now();
    let mut batches = 0usize;
    while started.elapsed() < Duration::from_secs(3) {
        match tokio::time::timeout(Duration::from_millis(300), subscription.next()).await {
            Ok(Some(_)) => batches += 1,
            Ok(None) => return (started.elapsed(), batches, false),
            Err(_) => return (started.elapsed(), batches, true),
        }
    }
    (started.elapsed(), batches, false)
}

/// 场景：老手机有加密历史，新手机配了对但没拿到钥匙，历史在房间里全解不开。返回假核心（确认卡片往它塞）、
/// Bot、房间和新手机的时间线订阅。数据目录直接留着，测试结束时一起没。
async fn phone_with_undecryptable_history() -> (
    (Phone, Phone),
    Home,
    ChatRoom,
    OwnedRoomId,
    Arc<wonton_client::TimelineSubscription>,
) {
    let old_dir = Box::leak(Box::new(tempfile::tempdir().unwrap()));
    let new_dir = Box::leak(Box::new(tempfile::tempdir().unwrap()));
    let old = Phone::open(old_dir.path());
    let new = Phone::open(new_dir.path());
    let home = Home::start(&[old.device_id()]).await;
    old_phone(&home, &old, true).await;
    let bot = home.bot("hermes", "Hermes").await;
    let room_id = accept_the_bots_encrypted_invite(&home, &old, &bot).await;
    let bot_room = bot.open_room(&room_id).await.unwrap();
    let mut old_timeline = Timeline::open(&old, room_id.as_str()).await;
    for body in HISTORY {
        bot_room.send_text(body, &[]).await.unwrap();
        old_timeline
            .until("旧手机读到加密历史", |item| {
                message_text(&item.content)
                    .filter(|text| *text == body)
                    .map(|_| ())
            })
            .await;
    }
    let code = invite(&old).await;
    pair(&new, code);
    new.signed_in().await;
    new.wait_chat("新手机同步到私聊", |view| {
        view.rooms
            .iter()
            .any(|room| room.room_id == room_id.as_str())
            .then_some(())
    })
    .await;
    new.wait_ready().await;
    let subscription = new
        .subscribe_timeline(TimelineTarget {
            room_id: room_id.as_str().to_owned(),
            thread_root: None,
        })
        .await
        .unwrap();
    paginate_backwards(&new, room_id.as_str());
    ((old, new), home, bot_room, room_id, subscription)
}

/// 行为：新手机没拿到密钥、房间里是解不开的历史时，时间线空闲下来（一秒几千批会把壳的主线程占死，输入都排不上），
/// 新消息到了照样送到，解不开的条目照实显示为解不开。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn an_undecryptable_timeline_settles_instead_of_flooding_the_shell() {
    use std::collections::HashSet;
    use std::time::{Duration, Instant};
    // 两台手机留着：核心一放，订阅就跟着没了。
    let (_phones, home, bot_room, room_id, subscription) = phone_with_undecryptable_history().await;

    // 解不开的历史照实送到：是解不开的条目，不是空白或假的正文。
    let mut seen: HashSet<String> = HashSet::new();
    let mut undecryptable = false;
    let deadline = Instant::now() + WAIT;
    while Instant::now() < deadline {
        let Ok(Some(update)) =
            tokio::time::timeout(Duration::from_millis(200), subscription.next()).await
        else {
            continue;
        };
        for item in update.diffs.iter().flat_map(diff_items) {
            seen.insert(item.id.clone());
            if let TimelineContent::Message { message } = &item.content
                && message.body == MessageContent::Undecryptable
            {
                undecryptable = true;
            }
        }
        if undecryptable {
            break;
        }
    }
    assert!(undecryptable, "解不开的历史没有送到");

    let (took, batches, quiet) = until_quiet(&subscription).await;
    println!("解不开的历史：排空用了 {took:?}，共 {batches} 批");
    assert!(quiet, "解不开的历史送来后 3 秒都没安静（{batches} 批）");
    assert!(batches <= 100, "解不开的历史送来后还发了 {batches} 批");

    // 确认卡片到了（这个房间的）：卡片的变化也是一次，之后同样该安静。等卡片用每轮新克隆的接收者去等
    // `changed()` 的话，这里会一直有货。
    let expires_at_ms = (std::time::SystemTime::now() + Duration::from_secs(600))
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    home.set_confirmations(
        vec![serde_json::json!({
            "id": "after-the-history",
            "state": {"type": "pending"},
            "title": "还要确认",
            "lines": [],
            "choices": [{"answer": "accept", "label": "好"}],
            "expires_at_ms": expires_at_ms,
            "origin": {"room_id": room_id.as_str()},
        })],
        vec![],
    );
    let (took, batches, quiet) = until_quiet(&subscription).await;
    println!("确认卡片：排空用了 {took:?}，共 {batches} 批");
    assert!(quiet, "确认卡片到了以后 3 秒都没安静（{batches} 批）");
    assert!(batches <= 100, "确认卡片到了以后还发了 {batches} 批");

    // 空闲不是不动了：新消息到了照样送到。
    bot_room
        .send_text("after the undecryptable history", &[])
        .await
        .unwrap();
    let deadline = Instant::now() + WAIT;
    let mut arrived = false;
    while Instant::now() < deadline && !arrived {
        let Ok(Some(update)) =
            tokio::time::timeout(Duration::from_millis(200), subscription.next()).await
        else {
            continue;
        };
        arrived = update
            .diffs
            .iter()
            .flat_map(diff_items)
            .any(|item| !seen.contains(&item.id));
    }
    assert!(arrived, "新消息没有送到");
}
