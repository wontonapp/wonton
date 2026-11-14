//! 长按消息的菜单（ADR-0073）：菜单项按发送状态和本账号的权限给；撤回对双方都生效，没权限的当场拒绝；转发照原样
//! 发到另一个房间。

use wonton_client::{
    Action, ChatFailure, CoreError, MessageContent, RoomMembership, SendState, TimelineContent,
    TimelineItem, TimelineMessage,
};
use wonton_matrix::testkit::eventually;
use wonton_matrix::{ChatRoom, MatrixSession, MessageBody, OwnedRoomId, OwnedUserId};
use wonton_protocol::MessageAction;
use wonton_protocol::{TimelineAction, TimelineTarget};

use super::support::{Home, OWNER, Phone, Timeline};
use super::{PNG, accept_the_bots_invite, bot_receives};

/// Bot 建一个不加密的群聊、邀请主人，主人加入。主人在里面是普通成员（power level 0）。
pub(super) async fn join_group_room(phone: &Phone, bot: &MatrixSession, name: &str) -> OwnedRoomId {
    let room_id = invite_to_group_room(phone, bot, name).await;
    join(phone, &room_id).await;
    room_id
}

/// Bot 建一个不加密的群聊、邀请主人，等主人收到邀请。历史对新加入的成员可见（`private_chat` 预设）。
pub(super) async fn invite_to_group_room(
    phone: &Phone,
    bot: &MatrixSession,
    name: &str,
) -> OwnedRoomId {
    let owner: OwnedUserId = OWNER.parse().unwrap();
    let room_id = bot
        .create_group_room(name, &owner)
        .await
        .expect("Bot 建群聊");
    phone
        .wait_chat(
            "收到群聊的邀请",
            membership(&room_id, RoomMembership::Invited),
        )
        .await;
    room_id
}

pub(super) async fn join(phone: &Phone, room_id: &OwnedRoomId) {
    phone
        .dispatch(Action::Chat {
            action: wonton_protocol::ChatAction::JoinRoom {
                room_id: room_id.to_string(),
            },
        })
        .unwrap();
    phone
        .wait_chat("加入群聊", membership(room_id, RoomMembership::Joined))
        .await;
}

fn membership(
    room_id: &OwnedRoomId,
    wanted: RoomMembership,
) -> impl FnMut(&wonton_client::ChatViewModel) -> Option<()> {
    let room_id = room_id.clone();
    move |view| {
        view.rooms
            .iter()
            .any(|row| row.room_id == room_id.as_str() && row.membership == wanted)
            .then_some(())
    }
}

/// 正文是 `body` 的文字消息：条目 ID 和消息。
pub(super) fn text_message(item: &TimelineItem, body: &str) -> Option<(String, TimelineMessage)> {
    match &item.content {
        TimelineContent::Message { message } if matches!(&message.body, MessageContent::Text { body: text, .. } if text == body) => {
            Some((item.id.clone(), message.clone()))
        }
        _ => None,
    }
}

/// 事件 ID 是 `event_id` 的消息。
pub(super) fn by_event(item: &TimelineItem, event_id: &str) -> Option<TimelineMessage> {
    match &item.content {
        TimelineContent::Message { message } if message.event_id.as_deref() == Some(event_id) => {
            Some(message.clone())
        }
        _ => None,
    }
}

/// 菜单项的种类，按菜单顺序。
pub(super) fn kinds(actions: &[MessageAction]) -> Vec<&'static str> {
    actions
        .iter()
        .map(|action| match action {
            MessageAction::Copy { .. } => "copy",
            MessageAction::Reply { .. } => "reply",
            MessageAction::Forward => "forward",
            MessageAction::Redact => "redact",
        })
        .collect()
}

/// 把 `user` 在房间里的权限升到 100（管理员）。
async fn promote(room: &ChatRoom, user: &str) {
    let mut levels = room
        .fetch_state_event_content("m.room.power_levels", "")
        .await
        .expect("取权限")
        .expect("房间有权限事件");
    levels["users"][user] = 100.into();
    room.send_state_event_raw("m.room.power_levels", "", levels)
        .await
        .expect("改权限");
}

/// 普通成员：自己的消息能撤回，别人的不能，撤回别人的当场被拒、那条还在；升为管理员后别人的也能撤回。撤回后双方都是
/// 已撤回。还没发出去的本地回显只有复制。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn the_menu_follows_power_and_a_redaction_reaches_both_sides() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    phone.signed_in().await;
    let bot = home.bot("hermes", "Hermes").await;
    let room_id = join_group_room(&phone, &bot, "Group").await;
    let room = room_id.to_string();
    let bot_room = bot.open_room(&room_id).await.expect("Bot 打开群聊");
    bot_room.send_text("Bot 说的话").await.expect("Bot 发文字");
    let mut timeline = Timeline::open(&phone, &room).await;

    rejected_echo_only_copies(&phone, &mut timeline, &room).await;
    let send = |body: &str| Action::Timeline {
        action: TimelineAction::SendText {
            target: TimelineTarget {
                room_id: room.clone(),
                thread_root: None,
            },
            body: body.to_owned(),
            reply_to: None,
        },
    };
    phone.dispatch(send("主人说的话")).unwrap();
    let (own_item, own) = timeline
        .until("自己的消息发出去", |item| {
            text_message(item, "主人说的话").filter(|(_, m)| m.event_id.is_some())
        })
        .await;
    let (bots_item, bots) = timeline
        .until("收到 Bot 的消息", |item| {
            text_message(item, "Bot 说的话")
        })
        .await;
    assert_eq!(kinds(&own.actions), ["copy", "reply", "forward", "redact"]);
    assert_eq!(kinds(&bots.actions), ["copy", "reply", "forward"]);

    let redact = |item_id: &str| Action::Timeline {
        action: TimelineAction::RedactMessage {
            target: TimelineTarget {
                room_id: room.clone(),
                thread_root: None,
            },
            item_id: item_id.to_owned(),
        },
    };
    let refused = phone.dispatch(redact(&bots_item));
    assert!(
        matches!(refused, Err(CoreError::InvalidInput { .. })),
        "普通成员撤回别人的消息应当场被拒：{refused:?}"
    );

    promote(&bot_room, OWNER).await;
    timeline
        .until("升为管理员后 Bot 的消息能撤回", |item| {
            text_message(item, "Bot 说的话")
                .filter(|(_, m)| m.actions.contains(&MessageAction::Redact))
        })
        .await;
    phone.dispatch(redact(&bots_item)).unwrap();
    phone.dispatch(redact(&own_item)).unwrap();

    let ids = [own.event_id.unwrap(), bots.event_id.unwrap()];
    timeline
        .until_items("两条在主人这边都撤回了", |items| {
            ids.iter()
                .all(|id| {
                    items
                        .iter()
                        .filter_map(|item| by_event(item, id))
                        .any(|m| m.body == MessageContent::Redacted && m.actions.is_empty())
                })
                .then_some(())
        })
        .await;
    eventually(
        "两条在 Bot 那边都撤回了",
        bot_room.message_updates().await,
        |messages| {
            ids.iter()
                .all(|id| {
                    messages.iter().any(|m| {
                        m.event_id.as_ref().map(|e| e.as_str()) == Some(id.as_str())
                            && m.body == MessageBody::Redacted
                    })
                })
                .then_some(())
        },
    )
    .await;
}

/// 服务器拒收的一条停在本地回显：没有事件 ID，菜单只有复制。取消它，后面的才发得出去。
async fn rejected_echo_only_copies(phone: &Phone, timeline: &mut Timeline, room: &str) {
    // 事件远超 65535 字节，Tuwunel 拒收（413 M_TOO_LARGE）。
    let too_long = "字".repeat(30_000);
    phone
        .dispatch(Action::Timeline {
            action: TimelineAction::SendText {
                target: TimelineTarget {
                    room_id: room.to_owned(),
                    thread_root: None,
                },
                body: too_long.clone(),
                reply_to: None,
            },
        })
        .unwrap();
    let (item_id, rejected) = timeline
        .until("太长的那条被拒收", |item| {
            text_message(item, &too_long)
                .filter(|(_, m)| matches!(m.send_state, SendState::Failed { .. }))
        })
        .await;
    assert_eq!(rejected.event_id, None);
    assert_eq!(kinds(&rejected.actions), ["copy"]);
    phone
        .dispatch(Action::Timeline {
            action: TimelineAction::CancelSend {
                target: TimelineTarget {
                    room_id: room.to_owned(),
                    thread_root: None,
                },
                item_id,
            },
        })
        .unwrap();
}

/// 转发文字和图片：目标房间里是同样的内容、不是回复，加密图片不重新上传、对方照样取得到。转发到没加入的房间、
/// 回复别的房间里的消息都按输入非法拒绝。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn forwarded_messages_arrive_as_they_were() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    phone.signed_in().await;
    let bot = home.bot("hermes", "Hermes").await;
    let direct = accept_the_bots_invite(&home, &phone, &bot).await;
    let group = join_group_room(&phone, &bot, "Group").await;
    let bot_direct = bot.open_room(&direct).await.expect("Bot 打开私聊");
    bot_direct.send_text("转发这句").await.expect("Bot 发文字");
    bot_direct
        .send_image(super::test_image())
        .await
        .expect("Bot 发图片");
    let mut timeline = Timeline::open(&phone, direct.as_str()).await;
    let (text_item, text) = timeline
        .until("收到要转发的文字", |item| {
            text_message(item, "转发这句")
        })
        .await;
    let (image_item, original) = timeline
        .until("收到要转发的图片", |item| match &item.content {
            TimelineContent::Message { message } => match &message.body {
                MessageContent::Image { image } => Some((item.id.clone(), image.clone())),
                _ => None,
            },
            _ => None,
        })
        .await;
    assert!(text.actions.contains(&MessageAction::Forward));

    let forward = |item_id: &str, to: &str| Action::Timeline {
        action: TimelineAction::ForwardMessage {
            target: TimelineTarget {
                room_id: direct.to_string(),
                thread_root: None,
            },
            item_id: item_id.to_owned(),
            to_room_id: to.to_owned(),
        },
    };
    let nowhere = phone.dispatch(forward(&text_item, "!nowhere:wonton.localhost"));
    assert!(
        matches!(nowhere, Err(CoreError::InvalidInput { .. })),
        "{nowhere:?}"
    );
    phone.dispatch(forward(&text_item, group.as_str())).unwrap();
    phone
        .dispatch(forward(&image_item, group.as_str()))
        .unwrap();

    let bot_group = bot.open_room(&group).await.expect("Bot 打开群聊");
    bot_receives(&bot_group, "群里收到转发的文字", |m| {
        (m.sender.as_str() == OWNER && m.body == MessageBody::Text("转发这句".to_owned()))
            .then(|| assert!(m.reply.is_none(), "转发的不是回复"))
    })
    .await;
    let forwarded = bot_receives(&bot_group, "群里收到转发的图片", |m| {
        match &m.body {
            MessageBody::Image(image) if m.sender.as_str() == OWNER => {
                assert!(m.reply.is_none(), "转发的不是回复");
                Some(image.clone())
            }
            _ => None,
        }
    })
    .await;
    assert_eq!(
        forwarded.media_token(),
        original.media.token,
        "没有重新上传"
    );
    let bytes = bot
        .download_media(&forwarded.media_token())
        .await
        .expect("Bot 取图片");
    assert_eq!(bytes, PNG);

    reply_to_another_room_is_invalid(&phone, group.as_str(), text.event_id.unwrap()).await;
}

/// 回复的消息不在这个房间里：发不出去，按输入非法进 `problem`。
async fn reply_to_another_room_is_invalid(phone: &Phone, room: &str, elsewhere: String) {
    phone
        .dispatch(Action::Timeline {
            action: TimelineAction::SendText {
                target: TimelineTarget {
                    room_id: room.to_owned(),
                    thread_root: None,
                },
                body: "引用别的房间".to_owned(),
                reply_to: Some(elsewhere),
            },
        })
        .unwrap();
    phone
        .wait_chat("回复别的房间的消息被拒", |view| {
            view.problem
                .as_ref()
                .filter(|problem| problem.reason == ChatFailure::InvalidInput)
                .map(drop)
        })
        .await;
}
