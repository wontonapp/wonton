//! 等回答的卡片（ADR-0016）：主人在别的房间时，核心也知道哪些卡片在等他，哪些已经不用答了。测试扮演核心账号，手机是真实的客户端核心。

use serde_json::json;
use wonton_client::{Action, Answer, PendingConfirmation};

use super::confirmation::{
    card, content, core_error, core_room, edit, in_ten_minutes, joined_direct_chat, now_ms, send,
    with_state,
};
use super::support::{Home, Phone, Timeline};

fn ids(pending: &[PendingConfirmation]) -> Vec<&str> {
    pending.iter().map(|card| card.id.as_str()).collect()
}

fn answer(phone: &Phone, id: &str) {
    phone
        .dispatch(Action::AnswerConfirmation {
            id: id.to_owned(),
            answer: Answer::Reject,
            choice_id: None,
            secrets: Default::default(),
        })
        .unwrap();
}

async fn wait_pending(phone: &Phone, what: &str, expected: &[&str]) {
    phone
        .wait_chat(what, |view| {
            (ids(&view.pending_confirmations) == expected).then_some(())
        })
        .await;
}

async fn wait_settled(phone: &Phone, what: &str, id: &str) {
    phone
        .wait_chat(what, |view| {
            view.settled_confirmations
                .iter()
                .any(|settled| settled == id)
                .then_some(())
        })
        .await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn cards_are_known_from_another_room_until_they_are_answered() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    phone.signed_in().await;
    let (core, notices) = core_room(&phone, &home).await;
    let room = core.open_room(&notices).await.expect("核心打开私聊");
    let hermes = home.bot("hermes", "Hermes").await;
    let chat = joined_direct_chat(&phone, &hermes).await;
    let _elsewhere = Timeline::open(&phone, chat.as_str()).await;

    let later = card("later", in_ten_minutes());
    send(&room, content(&later)).await;
    wait_pending(&phone, "第一张卡片", &["later"]).await;
    let view = phone.chat();
    assert_eq!(view.pending_confirmations[0].room_id, notices.as_str());
    assert_eq!(
        view.pending_confirmations[0].title,
        later["title"].as_str().unwrap()
    );
    send(&room, content(&card("sooner", now_ms() + 300_000))).await;
    wait_pending(&phone, "最早到期的在前", &["sooner", "later"]).await;

    // 回答发出去、还没有结果时，两边都不算。
    home.hold_answers(true);
    answer(&phone, "sooner");
    wait_pending(&phone, "正在回答的不用再催", &["later"]).await;
    assert!(phone.chat().settled_confirmations.is_empty());
    home.hold_answers(false);
    wait_settled(&phone, "答完了", "sooner").await;

    // 核心不收了（比如主人在另一台设备上答过）：不用再答。
    home.reply_to_next_answer(409, core_error("already_answered", "这个确认已经回答过了"));
    answer(&phone, "later");
    wait_settled(&phone, "别处答过了", "later").await;
    wait_pending(&phone, "没有卡片在等了", &[]).await;

    // 核心的编辑同样算数。
    let third = card("third", in_ten_minutes());
    let third_event = send(&room, content(&third)).await;
    wait_pending(&phone, "第三张卡片", &["third"]).await;
    edit(
        &room,
        &third_event,
        &with_state(&third, json!({"type": "running"})),
    )
    .await;
    wait_settled(&phone, "别的设备答过，核心改了卡片", "third").await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_card_that_runs_out_is_settled_without_an_edit() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    phone.signed_in().await;
    let (core, notices) = core_room(&phone, &home).await;
    let room = core.open_room(&notices).await.expect("核心打开私聊");

    send(&room, content(&card("short", now_ms() + 20_000))).await;
    wait_pending(&phone, "卡片在等", &["short"]).await;
    wait_settled(&phone, "到期了", "short").await;
    assert!(phone.chat().pending_confirmations.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_card_sent_while_the_app_was_closed_is_found_after_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    phone.signed_in().await;
    let (core, notices) = core_room(&phone, &home).await;
    let room = core.open_room(&notices).await.expect("核心打开私聊");
    phone
        .wait_chat("私聊标记同步好", |view| {
            view.rooms
                .iter()
                .any(|row| row.room_id == notices.as_str() && row.direct_user_id.is_some())
                .then_some(())
        })
        .await;
    drop(phone);

    let expires = in_ten_minutes();
    send(&room, content(&card("first", expires))).await;
    send(&room, content(&card("second", expires + 1))).await;
    let phone = Phone::open(dir.path());
    phone.connect(&home);
    wait_pending(&phone, "重启后还知道", &["first", "second"]).await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn the_same_card_twice_in_the_timeline_counts_once() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    phone.signed_in().await;
    let (core, notices) = core_room(&phone, &home).await;
    let room = core.open_room(&notices).await.expect("核心打开私聊");
    let expires = in_ten_minutes();
    send(&room, content(&card("same", expires))).await;
    send(&room, content(&card("same", expires))).await;
    send(&room, content(&card("other", expires + 1))).await;
    wait_pending(&phone, "重复的只算一张", &["same", "other"]).await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn the_notice_room_is_read_only_and_other_chats_are_not() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    phone.signed_in().await;
    let (_core, notices) = core_room(&phone, &home).await;
    let friend = home.bot("friend", "Friend").await;
    let chat = joined_direct_chat(&phone, &friend).await;
    let read_only = |room: &str| {
        let room = room.to_owned();
        move |view: &wonton_client::ChatViewModel| {
            view.rooms
                .iter()
                .find(|row| row.room_id == room && row.direct_user_id.is_some())
                .map(|row| row.is_read_only)
        }
    };
    assert!(
        phone
            .wait_chat("通知房间只读", read_only(notices.as_str()))
            .await
    );
    assert!(
        !phone
            .wait_chat("别的私聊能发", read_only(chat.as_str()))
            .await
    );

    let send = |room_id: &str| {
        phone.dispatch(Action::SendText {
            room_id: room_id.to_owned(),
            body: "你好".to_owned(),
            reply_to: None,
        })
    };
    assert!(send(notices.as_str()).is_err());
    assert!(send(chat.as_str()).is_ok());
}
