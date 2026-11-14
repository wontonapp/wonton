//! 确认卡片（ADR-0016）：卡片不再发进 Matrix 房间。时间线增量里的卡片行来自核心的“等你确认”列表：
//! 发起会话等于这个房间的卡片。测试用假核心的确认接口塞卡片，用真的 Tuwunel 开房间断言每个房间的显示。

use std::time::{SystemTime, UNIX_EPOCH};

use wonton_client::{
    Action, Answer, ConfirmationState, MessageContent, RoomMembership, TimelineContent,
};
use wonton_matrix::{ChatRoom, MatrixSession, OwnedRoomId, OwnedUserId};

use super::support::{Home, OWNER, Phone, Timeline};

const CARD_FIELD: &str = "app.wonton.confirmation";

fn card(id: &str, room: &str, thread: Option<&str>) -> serde_json::Value {
    let mut card = json_card(id);
    let mut origin = serde_json::json!({"room_id": room});
    if let Some(thread) = thread {
        origin["thread_id"] = serde_json::json!(thread);
    }
    card["origin"] = origin;
    card
}

fn json_card(id: &str) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "state": {"type": "pending"},
        "title": format!("{id} 要确认"),
        "lines": [],
        "choices": [
            {"answer": "approve", "label": "允许"},
            {"answer": "reject", "label": "拒绝"},
        ],
        "expires_at_ms": in_ten_minutes(),
    })
}

pub(crate) fn in_ten_minutes() -> u64 {
    (SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64)
        + 600_000
}

/// 旧版核心发在 Wonton 私聊里的卡片消息内容（`wonton-core` 的 `cards::content`）。
fn old_card_message(id: &str) -> serde_json::Value {
    serde_json::json!({
        "msgtype": "m.text",
        "body": format!("{id}：这是发给其他客户端看的说明"),
        CARD_FIELD: json_card(id),
    })
}

pub(crate) async fn send(room: &ChatRoom, content: serde_json::Value) {
    room.send_raw_message(content).await.expect("发消息");
}

fn answer(phone: &Phone, id: &str, answer: Answer) {
    phone
        .dispatch(Action::Chat {
            action: wonton_protocol::ChatAction::AnswerConfirmation {
                id: id.to_owned(),
                answer,
                choice_id: None,
                secrets: Default::default(),
                settings: Default::default(),
                devices: Vec::new(),
            },
        })
        .unwrap();
}

fn ids(views: &[wonton_client::ConfirmationView]) -> Vec<String> {
    views.iter().map(|view| view.card.id.clone()).collect()
}

pub(crate) async fn core_room(phone: &Phone, home: &Home) -> (MatrixSession, OwnedRoomId) {
    let core = home.bot("wonton-core", "Wonton").await;
    let room_id = joined_direct_chat(phone, &core).await;
    (core, room_id)
}

/// 不加密的私聊，主人接受邀请。
pub(crate) async fn joined_direct_chat(phone: &Phone, peer: &MatrixSession) -> OwnedRoomId {
    let owner: OwnedUserId = OWNER.parse().unwrap();
    let room_id = peer
        .create_direct_chat(&owner, false)
        .await
        .expect("发起私聊");
    phone
        .wait_chat("收到邀请", |view| {
            view.rooms
                .iter()
                .any(|row| {
                    row.room_id == room_id.as_str() && row.membership == RoomMembership::Invited
                })
                .then_some(())
        })
        .await;
    phone
        .dispatch(Action::Chat {
            action: wonton_protocol::ChatAction::JoinRoom {
                room_id: room_id.to_string(),
            },
        })
        .unwrap();
    phone
        .wait_chat("加入了私聊", |view| {
            view.rooms
                .iter()
                .any(|row| {
                    row.room_id == room_id.as_str() && row.membership == RoomMembership::Joined
                })
                .then_some(())
        })
        .await;
    room_id
}

/// 每个房间只显示发起会话是自己的卡片：Bot 房间里有它的，别的房间里没有；话题发起的也算这个房间。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_room_shows_only_the_cards_started_in_it() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    phone.signed_in().await;

    let hermes = home.bot("hermes", "Hermes").await;
    let hermes_room = joined_direct_chat(&phone, &hermes).await;
    let grafana = home.bot("grafana", "Grafana").await;
    let grafana_room = joined_direct_chat(&phone, &grafana).await;

    home.set_confirmations(
        vec![
            card("install", hermes_room.as_str(), None),
            card("tune", hermes_room.as_str(), Some("$thread-root")),
            card("query", grafana_room.as_str(), None),
        ],
        vec![],
    );
    let mut timeline = Timeline::open(&phone, hermes_room.as_str()).await;
    let shown = timeline
        .until_confirmations("Hermes 房间的卡片", |views| {
            (views.len() == 2).then(|| ids(views))
        })
        .await;
    assert_eq!(shown, ["install", "tune"], "话题发起的也算这个房间");

    // 打开别的房间：那里只有它自己的卡片。
    let mut other = Timeline::open(&phone, grafana_room.as_str()).await;
    let shown = other
        .until_confirmations("Grafana 房间的卡片", |views| {
            (views.len() == 1).then(|| ids(views))
        })
        .await;
    assert_eq!(shown, ["query"]);
    drop(other);
    drop(timeline);
    let mut timeline = Timeline::open(&phone, hermes_room.as_str()).await;
    timeline
        .until_confirmations("回到 Hermes 房间", |views| {
            (views.len() == 2).then_some(())
        })
        .await;

    // 回答的进展在房间里有：发送中 → 结果。
    home.hold_answers(true);
    answer(&phone, "install", Answer::Approve);
    timeline
        .until_confirmations("正在发", |views| {
            views
                .iter()
                .find(|view| view.card.id == "install")
                .filter(|view| view.sending == Some(Answer::Approve))
                .map(|_| ())
        })
        .await;
    home.hold_answers(false);
    timeline
        .until_confirmations("核心收下回答", |views| {
            views
                .iter()
                .find(|view| view.card.id == "install")
                .filter(|view| view.card.state == ConfirmationState::Running)
                .map(|_| ())
        })
        .await;

    // 核心的列表把它挪进结束的：这个 App 看着它结束，房间里显示结果。
    home.set_confirmations(
        vec![card("tune", hermes_room.as_str(), Some("$thread-root"))],
        vec![serde_json::json!({
            "id": "install",
            "state": {"type": "running"},
            "title": "install 要确认",
            "lines": [],
            "choices": [],
            "expires_at_ms": in_ten_minutes(),
            "origin": {"room_id": hermes_room.as_str()},
        })],
    );
    timeline
        .until_confirmations("房间里显示结果", |views| {
            views
                .iter()
                .find(|view| view.card.id == "install")
                .filter(|view| view.card.state == ConfirmationState::Running)
                .map(|_| ())
        })
        .await;
}

/// 旧版核心发的卡片消息和别的消息一样按正文显示：没有按钮，也不进房间的卡片行。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn an_old_card_message_displays_as_a_normal_message() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    phone.signed_in().await;
    let (core, room_id) = core_room(&phone, &home).await;
    let room = core.open_room(&room_id).await.expect("核心打开私聊");
    send(&room, old_card_message("legacy")).await;

    let mut timeline = Timeline::open(&phone, room_id.as_str()).await;
    timeline
        .until("旧卡片按正文显示", |item| match &item.content {
            TimelineContent::Message { message } => match &message.body {
                MessageContent::Text { blocks, .. } => blocks
                    .iter()
                    .flat_map(|block| &block.spans)
                    .any(|span| span.text.contains("这是发给其他客户端看的说明"))
                    .then_some(()),
                _ => None,
            },
            _ => None,
        })
        .await;
    timeline
        .until_confirmations("房间里没有卡片行", |views| {
            views.is_empty().then_some(())
        })
        .await;
}
