//! “问 Bot”的目标解析和回执：家人发到本人 Bot 的主房间；主人、没有私人 Bot 的人发到默认 Bot；
//! 失败按原因写回执，不占全局 `problem`。

use serde_json::json;
use wonton_client::{Action, MessageContent, RoomMembership, TimelineContent};
use wonton_matrix::OwnedUserId;
use wonton_protocol::{AskFailure, AskState, ChatAction, HomeAction, OwnPrivateBotView};

use super::support::{Home, OWNER, Phone, Timeline};

fn tempdir() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

/// 引导里有一个可见的默认 Bot。
fn onboarding(default_bot: &str) -> serde_json::Value {
    json!({
        "step": "finished",
        "default_bot_plugin": {
            "id": "hermes",
            "bot_user_id": default_bot,
            "visibility": "visible"
        },
        "needs": [],
        "prompts": [],
        "catalog_problem": null
    })
}

/// 主人的手机先登录。登录会建出主人的 Matrix 账号，Bot 才能和它建私聊。
async fn signed_in(home: &Home, phone: &Phone) {
    home.set_onboarding(onboarding("@hermes:wonton.localhost"));
    let _owner = home.bot("owner", "Owner").await;
    phone.connect(home);
    phone.signed_in().await;
}

/// 换一份本人 Bot 的读取结果并让核心重新取一次（登录时接口还是 404）。
fn set_own_bot(home: &Home, phone: &Phone, own: serde_json::Value) {
    home.set_own_private_bot(own);
    phone
        .dispatch(Action::Home {
            action: HomeAction::AppForegrounded,
        })
        .unwrap();
}

/// 等本人 Bot 的读取结果进 ViewModel。
async fn wait_own_bot(phone: &Phone, ready: bool) {
    phone
        .wait_chat("本人 Bot 的读取结果", |view| {
            (matches!(&view.own_private_bot, Some(OwnPrivateBotView::Ready { .. }))
                || (!ready && view.own_private_bot.is_some()))
            .then_some(())
        })
        .await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn family_ask_goes_to_the_own_bot_home_room() {
    let dir = tempdir();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    signed_in(&home, &phone).await;
    let bot = home.bot("user-mia", "Mia 的 Bot").await;
    let owner: OwnedUserId = OWNER.parse().unwrap();
    let room = bot.create_direct_chat(&owner, false).await.unwrap();
    set_own_bot(
        &home,
        &phone,
        json!({
            "requester_user_id": OWNER,
            "result": {
                "state": "ready",
                "bot": {"plugin_id": "app.hermes", "bot": "user-mia"},
                "user_id": bot.user_id(),
                "home_room": room.to_string(),
                "room_problem": null
            }
        }),
    );
    wait_own_bot(&phone, true).await;

    phone
        .dispatch(Action::Chat {
            action: ChatAction::AskAgent {
                text: "  在吗 ".to_owned(),
                room_id: None,
                request_id: Some(7),
            },
        })
        .unwrap();
    phone
        .wait_chat("回执指向本人 Bot 的主房间", |view| {
            view.ask
                .as_ref()
                .is_some_and(|receipt| {
                    receipt.request_id == Some(7)
                        && receipt.state == AskState::Sent
                        && receipt.room_id.as_deref() == Some(room.as_str())
                })
                .then_some(())
        })
        .await;
    // 壳打开的就是回执房间；正文去掉首尾空白。
    let mut timeline = Timeline::open(&phone, room.as_str()).await;
    timeline
        .until("问题发进了本人 Bot 的主房间", |item| match &item.content {
            TimelineContent::Message { message } => match &message.body {
                MessageContent::Text { body, .. } if body == "在吗" => Some(()),
                _ => None,
            },
            _ => None,
        })
        .await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn family_without_a_ready_bot_fails_with_no_bot() {
    let dir = tempdir();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    signed_in(&home, &phone).await;
    let bot = home.bot("hermes", "Hermes").await;
    let owner: OwnedUserId = OWNER.parse().unwrap();
    let _room = bot.create_direct_chat(&owner, false).await.unwrap();
    set_own_bot(
        &home,
        &phone,
        json!({"requester_user_id": OWNER, "result": {"state": "absent"}}),
    );
    wait_own_bot(&phone, false).await;

    phone
        .dispatch(Action::Chat {
            action: ChatAction::AskAgent {
                text: "在吗".to_owned(),
                room_id: None,
                request_id: Some(8),
            },
        })
        .unwrap();
    phone
        .wait_chat("回执说没有可用的 Bot", |view| {
            view.ask
                .as_ref()
                .is_some_and(|receipt| {
                    receipt.request_id == Some(8)
                        && receipt.state == AskState::Failed
                        && receipt.room_id.is_none()
                        && receipt
                            .problem
                            .as_ref()
                            .is_some_and(|problem| problem.reason == AskFailure::NoBot)
                })
                .then_some(())
        })
        .await;
    assert!(phone.core.chat().problem.is_none());
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn own_bot_room_not_joined_is_not_ready() {
    let dir = tempdir();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    signed_in(&home, &phone).await;
    let bot = home.bot("user-mia", "Mia 的 Bot").await;
    let owner: OwnedUserId = OWNER.parse().unwrap();
    let _room = bot.create_direct_chat(&owner, false).await.unwrap();
    set_own_bot(
        &home,
        &phone,
        json!({
            "requester_user_id": OWNER,
            "result": {
                "state": "ready",
                "bot": {"plugin_id": "app.hermes", "bot": "user-mia"},
                "user_id": bot.user_id(),
                "home_room": null,
                "room_problem": "not_joined"
            }
        }),
    );
    wait_own_bot(&phone, true).await;
    phone
        .dispatch(Action::Chat {
            action: ChatAction::AskAgent {
                text: "在吗".to_owned(),
                room_id: None,
                request_id: Some(9),
            },
        })
        .unwrap();
    phone
        .wait_chat("回执说 Bot 还没就绪", |view| {
            view.ask
                .as_ref()
                .is_some_and(|receipt| {
                    receipt.state == AskState::Failed
                        && receipt
                            .problem
                            .as_ref()
                            .is_some_and(|problem| problem.reason == AskFailure::BotNotReady)
                })
                .then_some(())
        })
        .await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn contact_required_is_its_own_reason_and_keeps_the_room() {
    let dir = tempdir();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    signed_in(&home, &phone).await;
    let bot = home.bot("user-mia", "Mia 的 Bot").await;
    let owner: OwnedUserId = OWNER.parse().unwrap();
    let room = bot.create_direct_chat(&owner, false).await.unwrap();
    home.refuse_matrix("/send/", 403, "APP.WONTON.CONTACT_REQUIRED");
    set_own_bot(
        &home,
        &phone,
        json!({
            "requester_user_id": OWNER,
            "result": {
                "state": "ready",
                "bot": {"plugin_id": "app.hermes", "bot": "user-mia"},
                "user_id": bot.user_id(),
                "home_room": room.to_string(),
                "room_problem": null
            }
        }),
    );
    wait_own_bot(&phone, true).await;
    phone
        .dispatch(Action::Chat {
            action: ChatAction::AskAgent {
                text: "在吗".to_owned(),
                room_id: None,
                request_id: Some(10),
            },
        })
        .unwrap();
    phone
        .wait_chat("回执说对方还不是联系人", |view| {
            view.ask
                .as_ref()
                .is_some_and(|receipt| {
                    receipt.state == AskState::Failed
                        && receipt.room_id.as_deref() == Some(room.as_str())
                        && receipt
                            .problem
                            .as_ref()
                            .is_some_and(|problem| problem.reason == AskFailure::ContactRequired)
                })
                .then_some(())
        })
        .await;
    assert!(phone.core.chat().problem.is_none());
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_read_only_room_is_not_a_target() {
    let dir = tempdir();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    signed_in(&home, &phone).await;
    let core = home.bot("wonton-core", "Wonton").await;
    let owner: OwnedUserId = OWNER.parse().unwrap();
    let room = core.create_direct_chat(&owner, false).await.unwrap();
    set_own_bot(
        &home,
        &phone,
        json!({"requester_user_id": OWNER, "result": {"state": "not_applicable"}}),
    );
    phone
        .wait_chat("通知房先出房行", |view| {
            view.rooms
                .iter()
                .any(|row| row.room_id == room && row.is_read_only)
                .then_some(())
        })
        .await;
    phone
        .dispatch(Action::Chat {
            action: ChatAction::AskAgent {
                text: "在吗".to_owned(),
                room_id: Some(room.to_string()),
                request_id: Some(11),
            },
        })
        .unwrap();
    phone
        .wait_chat("回执说这是只读的通知房", |view| {
            view.ask
                .as_ref()
                .is_some_and(|receipt| {
                    receipt.state == AskState::Failed
                        && receipt.room_id.as_deref() == Some(room.as_str())
                        && receipt
                            .problem
                            .as_ref()
                            .is_some_and(|problem| problem.reason == AskFailure::ReadOnlyRoom)
                })
                .then_some(())
        })
        .await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn an_owner_without_a_private_bot_asks_the_default_bot() {
    let dir = tempdir();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    signed_in(&home, &phone).await;
    let bot = home.bot("hermes", "Hermes").await;
    let owner: OwnedUserId = OWNER.parse().unwrap();
    let room = bot.create_direct_chat(&owner, false).await.unwrap();
    set_own_bot(
        &home,
        &phone,
        json!({"requester_user_id": OWNER, "result": {"state": "not_applicable"}}),
    );
    phone
        .wait_chat("默认 Bot 的私聊先出房行", |view| {
            view.rooms
                .iter()
                .any(|row| {
                    row.room_id == room
                        && row.direct_user_id.as_deref() == Some(bot.user_id().as_str())
                        && row.membership == RoomMembership::Invited
                })
                .then_some(())
        })
        .await;
    phone
        .dispatch(Action::Chat {
            action: ChatAction::AskAgent {
                text: "在吗".to_owned(),
                room_id: None,
                request_id: Some(12),
            },
        })
        .unwrap();
    phone
        .wait_chat("回执指向默认 Bot 的房间", |view| {
            view.ask
                .as_ref()
                .is_some_and(|receipt| {
                    receipt.state == AskState::Sent
                        && receipt.room_id.as_deref() == Some(room.as_str())
                })
                .then_some(())
        })
        .await;
}
