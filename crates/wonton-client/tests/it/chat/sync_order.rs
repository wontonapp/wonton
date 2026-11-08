//! 取消在飞请求后，旧请求仍可经独立隧道流迟到；订阅断言必须按本地入口顺序检查，不能按服务端记录顺序。

use std::time::Duration;

use super::confirmation::joined_direct_chat;
use super::support::{Home, Phone, Timeline, WAIT, is_room_list};
use serde_json::Value;

async fn wait_request(home: &Home, check: impl Fn(&Value) -> bool) -> Value {
    tokio::time::timeout(WAIT, async {
        loop {
            if let Some(request) = home.sync_requests().into_iter().find(&check) {
                return request;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("expected sync request must be recorded")
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_cancelled_empty_request_can_arrive_after_its_subscribed_replacement() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    phone.signed_in().await;
    let bot = home.bot("hermes", "Hermes").await;
    let room_id = joined_direct_chat(&phone, &bot).await;
    let bot_room = bot.open_room(&room_id).await.unwrap();
    home.hold_next_unsubscribed_sync_recording();
    bot_room
        .send_text("start the next long poll")
        .await
        .unwrap();
    let old = home.held_sync_request().await;
    assert!(is_room_list(&old["conn_id"]), "{}", old["conn_id"]);
    assert!(old.get("room_subscriptions").is_none());
    let old_sequence = old["test_sequence"].as_u64().unwrap();
    let timeline = Timeline::open(&phone, room_id.as_str()).await;
    let new = wait_request(&home, |request| {
        is_room_list(&request["conn_id"])
            && request["test_sequence"].as_u64().unwrap() > old_sequence
            && request["room_subscriptions"]
                .get(room_id.as_str())
                .is_some()
    })
    .await;
    assert_eq!(
        old["pos"], new["pos"],
        "replacement must reuse the cancelled position"
    );
    let new_sequence = new["test_sequence"].as_u64().unwrap();
    assert!(old_sequence < new_sequence, "old={old:?}; new={new:?}");
    home.release_sync_recording();
    wait_request(&home, |request| request["test_sequence"] == old_sequence).await;
    let arrivals = home.sync_arrivals();
    let old_at = arrivals
        .iter()
        .position(|request| request["test_sequence"] == old_sequence)
        .unwrap();
    let new_at = arrivals
        .iter()
        .position(|request| request["test_sequence"] == new_sequence)
        .unwrap();
    assert!(
        new_at < old_at,
        "replacement must overtake old request: {arrivals:?}"
    );
    let ordered = home.sync_requests();
    let old_at = ordered
        .iter()
        .position(|request| request["test_sequence"] == old_sequence)
        .unwrap();
    let new_at = ordered
        .iter()
        .position(|request| request["test_sequence"] == new_sequence)
        .unwrap();
    assert!(
        old_at < new_at,
        "source ordering must retain both exact requests: {ordered:?}"
    );
    drop(timeline);
}
