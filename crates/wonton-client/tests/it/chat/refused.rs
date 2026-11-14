//! 服务器不让做某件事（没有权限、对方屏蔽了自己、对方的服务器不和这台互通）时 Matrix 回 403：登录还有效，
//! 不能报成登录失效。只有 401，或 403 带令牌、账号失效的错误码，才是登录失效。后台操作的类别在 `ChatProblem`
//! 里，直接返回给壳的调用（取媒体、解析房间链接）在 `CoreError` 里。

use wonton_client::{
    Action, ChatAccount, ChatFailure, ChatProblem, ChatSync, CoreError, MessageContent,
    RoomMembership, SendState, TimelineContent,
};
use wonton_protocol::{TimelineAction, TimelineTarget};

use super::support::{Home, OWNER, Phone, Timeline};
use super::{accept_the_bots_invite, bots_image, test_image};

const CREATE_ROOM: &str = "post /_matrix/client/v3/createroom";
const REFRESH: &str = "post /_matrix/client/v3/refresh";
/// 认证媒体（`/_matrix/client/v1/media/download/…`）和旧的媒体接口都含这一段。
const MEDIA_DOWNLOAD: &str = "/download/";

async fn signed_in() -> (tempfile::TempDir, Home, Phone) {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    phone.signed_in().await;
    (dir, home, phone)
}

async fn start_direct_chat(home: &Home, phone: &Phone) {
    let bob = home.bot("bob", "Bob").await;
    phone
        .dispatch(Action::Chat {
            action: wonton_protocol::ChatAction::StartDirectChat {
                user_id: bob.user_id().to_string(),
            },
        })
        .unwrap();
}

/// Bot 邀请主人进私聊，等手机看到这条邀请。
async fn invited_by_the_bot(home: &Home, phone: &Phone) -> wonton_matrix::OwnedRoomId {
    let bot = home.bot("hermes", "Hermes").await;
    let owner = wonton_matrix::OwnedUserId::try_from(OWNER).unwrap();
    let room_id = bot
        .create_direct_chat(&owner, true)
        .await
        .expect("Bot 发起私聊");
    phone
        .wait_chat("收到 Bot 的邀请", |view| {
            view.rooms
                .iter()
                .any(|row| {
                    row.room_id == room_id.as_str() && row.membership == RoomMembership::Invited
                })
                .then_some(())
        })
        .await;
    room_id
}

async fn problem(phone: &Phone, what: &str) -> ChatProblem {
    phone.wait_chat(what, |view| view.problem.clone()).await
}

fn assert_still_signed_in(phone: &Phone) {
    let view = phone.chat();
    assert!(
        matches!(view.account, ChatAccount::SignedIn { .. }),
        "登录不该受影响：{:?}",
        view.account
    );
    assert_eq!(view.sync, ChatSync::Running);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_refused_direct_chat_is_not_reported_as_an_expired_login() {
    let (_dir, home, phone) = signed_in().await;
    home.refuse_matrix(CREATE_ROOM, 403, "M_FORBIDDEN");
    start_direct_chat(&home, &phone).await;
    let problem = problem(&phone, "发起私聊被拒").await;
    assert_eq!(problem.reason, ChatFailure::Forbidden, "{problem:?}");
    assert_still_signed_in(&phone);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_refused_join_is_not_reported_as_an_expired_login() {
    let (_dir, home, phone) = signed_in().await;
    let room_id = invited_by_the_bot(&home, &phone).await;
    home.refuse_matrix("/join", 403, "M_FORBIDDEN");
    phone
        .dispatch(Action::Chat {
            action: wonton_protocol::ChatAction::JoinRoom {
                room_id: room_id.to_string(),
            },
        })
        .unwrap();
    let problem = problem(&phone, "加入被拒").await;
    assert_eq!(problem.reason, ChatFailure::Forbidden, "{problem:?}");
    assert_still_signed_in(&phone);
}

/// 发送走 SDK 的发送队列：被拒的一条停在时间线上，不变成整页的问题，更不是登录失效。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_refused_message_stays_on_the_message_and_keeps_the_login() {
    let (_dir, home, phone) = signed_in().await;
    let bot = home.bot("hermes", "Hermes").await;
    let room = accept_the_bots_invite(&home, &phone, &bot)
        .await
        .to_string();
    let mut timeline = Timeline::open(&phone, &room).await;
    home.refuse_matrix("/send/m.room.encrypted/", 403, "M_FORBIDDEN");
    phone
        .dispatch(Action::Timeline {
            action: TimelineAction::SendText {
                target: TimelineTarget {
                    room_id: room.clone(),
                    thread_root: None,
                },
                body: "发不出去".to_owned(),
                reply_to: None,
            },
        })
        .unwrap();
    timeline
        .until("被拒的那条停在时间线上", |item| match &item.content {
            TimelineContent::Message { message }
                if message.is_own
                    && matches!(&message.body, MessageContent::Text { body, .. } if body == "发不出去")
                    && matches!(
                        message.send_state,
                        SendState::Failed {
                            recoverable: false,
                            ..
                        }
                    ) =>
            {
                Some(())
            }
            _ => None,
        })
        .await;
    assert_still_signed_in(&phone);
    assert_eq!(phone.chat().problem, None);
}

/// 令牌本身被拒（刷新也被拒）才是登录失效，401 和 403 一样。
async fn a_rejected_token_signs_the_chat_out(status: u16) {
    let (_dir, home, phone) = signed_in().await;
    home.refuse_matrix(CREATE_ROOM, status, "M_UNKNOWN_TOKEN");
    home.refuse_matrix(REFRESH, status, "M_UNKNOWN_TOKEN");
    start_direct_chat(&home, &phone).await;
    phone
        .wait_chat("登录失效", |view| {
            matches!(
                view.account,
                ChatAccount::Failed {
                    reason: ChatFailure::Unauthorized,
                    ..
                }
            )
            .then_some(())
        })
        .await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_403_unknown_token_still_signs_the_chat_out() {
    a_rejected_token_signs_the_chat_out(403).await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_401_unknown_token_still_signs_the_chat_out() {
    a_rejected_token_signs_the_chat_out(401).await;
}

/// Bot 在私聊里发一张图，等手机的时间线收到，返回它的媒体句柄。
async fn bots_image_in_room(home: &Home, phone: &Phone) -> wonton_client::MediaHandle {
    let bot = home.bot("hermes", "Hermes").await;
    let room_id = accept_the_bots_invite(home, phone, &bot).await;
    let mut timeline = Timeline::open(phone, room_id.as_str()).await;
    let bot_room = bot.open_room(&room_id).await.expect("Bot 打开私聊");
    bot_room.send_image(test_image()).await.expect("Bot 发图片");
    timeline.until("收到 Bot 的图片", bots_image).await.media
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_refused_media_download_is_not_reported_as_an_expired_login() {
    let (_dir, home, phone) = signed_in().await;
    let media = bots_image_in_room(&home, &phone).await;
    home.refuse_matrix(MEDIA_DOWNLOAD, 403, "M_FORBIDDEN");
    let error = phone.load_media(media).await.expect_err("取图片被拒");
    assert!(matches!(error, CoreError::Forbidden { .. }), "{error:?}");
    assert_still_signed_in(&phone);
}

/// 令牌失效时取媒体还是登录失效：调用报 `Unauthorized`，账号退出登录。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_media_download_with_a_rejected_token_still_signs_the_chat_out() {
    let (_dir, home, phone) = signed_in().await;
    let media = bots_image_in_room(&home, &phone).await;
    home.refuse_matrix(MEDIA_DOWNLOAD, 403, "M_UNKNOWN_TOKEN");
    home.refuse_matrix(REFRESH, 403, "M_UNKNOWN_TOKEN");
    let error = phone.load_media(media).await.expect_err("令牌被拒");
    assert!(matches!(error, CoreError::Unauthorized { .. }), "{error:?}");
    phone
        .wait_chat("登录失效", |view| {
            matches!(
                view.account,
                ChatAccount::Failed {
                    reason: ChatFailure::Unauthorized,
                    ..
                }
            )
            .then_some(())
        })
        .await;
}

/// 指向还没加入的房间的链接打不开，但登录好好的。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_link_to_a_room_not_joined_is_refused_not_an_expired_login() {
    let (_dir, home, phone) = signed_in().await;
    let room_id = invited_by_the_bot(&home, &phone).await;
    let error = phone
        .resolve_chat_room(room_id.to_string())
        .await
        .expect_err("没加入的房间");
    assert!(matches!(error, CoreError::Forbidden { .. }), "{error:?}");
    assert_still_signed_in(&phone);
}
