//! 聊天的行为测试：只经导出给原生壳的接口（`ClientCore`、`Action`、订阅）驱动，手机经隧道和本地代理登录主人
//! 的 Matrix 账号、和 Bot 聊天，重启后会话和加密都还在。
//!
//! 每个测试用 Docker 起一台一次性的 Tuwunel，所以标了 `#[ignore]`：
//! `cargo test -p wonton-client --test it chat:: -- --include-ignored`（`scripts/check.sh` 有 Docker 时自动运行）。

mod ask_agent;
mod bot_profile;
mod bot_profile_support;
mod card_plugin_refresh;
mod confirmation;
mod device_name;
mod key_upload_order;
mod onboarding;
mod pending;
mod push;
mod push_method;
mod rebuild_reason;
mod recovery;
mod relogin;
mod resume;
mod send;
mod server_switch;
mod stale_join;
mod subscriptions;
mod support;
mod sync_order;
mod timeline;
mod upload_progress;
mod verification;

use std::time::{Duration, Instant};

use wonton_client::{
    Action, ChatAccount, ImageContent, ImageThumbnail, ImageUpload, MessageContent, RecoveryStatus,
    RoomMembership, SecretString, TimelineContent, TimelineItem,
};
use wonton_matrix::testkit::eventually;
use wonton_matrix::{ChatRoom, MatrixSession, Message, MessageBody, OwnedRoomId, OwnedUserId};

use support::{Home, OWNER, Phone, Timeline, WAIT};

/// 2×2 的 PNG。
const PNG: &[u8] = include_bytes!("../../../../wonton-matrix/tests/tuwunel/fixtures/2x2.png");
/// 当缩略图用的字节。媒体的内容对 Matrix 不透明，测试只比对取回的字节。
const THUMBNAIL: &[u8] = b"a smaller preview of the PNG";

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn the_owner_chats_with_a_bot_through_the_tunnel_and_keeps_it_after_restart() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);

    // 连上家里的服务器后自动登录：经隧道向核心要登录令牌，用它登录主人的账号。
    let (user_id, device_id) = phone.signed_in().await;
    assert_eq!(user_id, OWNER);
    assert_eq!(home.login_tokens(), 1);

    let bot = home.bot("hermes", "Hermes").await;
    let room_id = accept_the_bots_invite(&home, &phone, &bot).await;
    chat_with_the_bot(&phone, &bot, &room_id).await;
    let recovery_key = read_and_wait_for_recovery(&phone, &room_id).await;
    drop(phone);

    // 重启：用同一个数据目录，恢复上次的会话，不再要登录令牌；加密的历史仍能解密。主人还没确认保存恢复密钥，
    // 所以还能看到它；确认后从 ViewModel 和本机都清掉，再重启也不会出现。
    let phone = Phone::open(dir.path());
    let (_, restored) = phone.signed_in().await;
    assert_eq!(restored, device_id, "重启后应是同一台设备");
    assert_eq!(home.login_tokens(), 1, "恢复会话不应再要登录令牌");
    assert_eq!(phone.chat().encryption.recovery_key, Some(recovery_key));
    let mut timeline = Timeline::open(&phone, room_id.as_str()).await;
    timeline
        .until("重启后仍能解密历史", |item| {
            text(item, "你好，主人")
        })
        .await;
    phone.dispatch(Action::ConfirmRecoveryKeySaved).unwrap();
    assert_eq!(phone.chat().encryption.recovery_key, None);
    drop(timeline);
    drop(phone);

    let phone = Phone::open(dir.path());
    phone.signed_in().await;
    let encryption = phone
        .wait_chat("重启后恢复仍是开启的", |view| {
            (view.encryption.recovery == RecoveryStatus::Enabled).then(|| view.encryption.clone())
        })
        .await;
    assert_eq!(
        encryption.recovery_key, None,
        "确认保存后不应再显示恢复密钥"
    );
    drop(phone);

    // 本机的聊天存储丢了（比如只恢复了一部分 App 数据）：登录状态单独留着也不能用，重新要令牌登录成一台新设备，
    // 不拿旧的设备 ID 配一套新的设备密钥。
    std::fs::remove_dir_all(dir.path().join("servers/1/matrix/store")).expect("删掉聊天存储");
    let phone = Phone::open(dir.path());
    let (_, fresh) = phone.signed_in().await;
    assert_ne!(fresh, device_id, "存储丢了应登录成新设备");
    assert_eq!(home.login_tokens(), 2, "存储丢了应重新要登录令牌");

    // 新设备解不开以前的加密消息（要用恢复密钥），但发送者照样显示名字：同步只带了最新一条的发送者，其余的
    // 成员资料在打开房间后从服务器补上。
    phone
        .wait_chat("新设备同步到私聊", |view| {
            view.rooms
                .iter()
                .any(|row| row.room_id == room_id.as_str())
                .then_some(())
        })
        .await;
    let mut timeline = Timeline::open(&phone, room_id.as_str()).await;
    // 新设备上只同步到了最新一条，像界面那样向前翻一页。
    phone
        .dispatch(Action::PaginateBackwards {
            room_id: room_id.to_string(),
        })
        .unwrap();
    timeline
        .until(
            "新设备上 Bot 的消息显示 Bot 的名字",
            |item| match &item.content {
                TimelineContent::Message { message }
                    if message.sender_id == bot.user_id().as_str()
                        && message.sender_name == "Hermes" =>
                {
                    Some(())
                }
                _ => None,
            },
        )
        .await;
}

/// 新设备打开房间时取不到成员，隔一会儿再取，Bot 的名字最终补上。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_new_device_gets_the_bots_name_even_if_the_first_member_requests_fail() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    phone.signed_in().await;
    let bot = home.bot("hermes", "Hermes").await;
    let room_id = accept_the_bots_invite(&home, &phone, &bot).await;
    chat_with_the_bot(&phone, &bot, &room_id).await;
    drop(phone);
    std::fs::remove_dir_all(dir.path().join("servers/1/matrix/store")).expect("删掉聊天存储");

    home.set_members_down(true);
    let phone = Phone::open(dir.path());
    phone.signed_in().await;
    phone
        .wait_chat("新设备同步到私聊", |view| {
            view.rooms
                .iter()
                .any(|row| row.room_id == room_id.as_str())
                .then_some(())
        })
        .await;
    let mut timeline = Timeline::open(&phone, room_id.as_str()).await;
    // SDK 取一次成员发 3 个请求就放弃（matrix-sdk 0.19.1 实测），第 4 个说明又取了一次。
    let deadline = Instant::now() + Duration::from_secs(60);
    while home.members_requests_dropped() < 4 {
        assert!(
            Instant::now() < deadline,
            "取成员失败以后没有再取，断开过 {} 个请求",
            home.members_requests_dropped()
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    home.set_members_down(false);
    phone
        .dispatch(Action::PaginateBackwards {
            room_id: room_id.to_string(),
        })
        .unwrap();
    timeline
        .until("Bot 的消息显示 Bot 的名字", |item| {
            match &item.content {
                TimelineContent::Message { message }
                    if message.sender_id == bot.user_id().as_str()
                        && message.sender_name == "Hermes" =>
                {
                    Some(())
                }
                _ => None,
            }
        })
        .await;
}

/// Bot 建私聊前等主人当前设备的密钥上服务器：Bot 发消息时的收信人设备表来自它本地 crypto store，
/// 手机的密钥还没上服务器它就查不到这台设备，首条消息的房间密钥不会发给手机，消息永远解不开。
/// 以服务器为准（`/keys/query`），没有就等假核心放行下一次 keys/upload 再查（事件驱动，不睡固定间隔）。
async fn wait_for_owner_device_keys(home: &Home, phone: &Phone, bot: &MatrixSession) {
    home.mark_device_keys_wait_started();
    let device_id = match &phone.chat().account {
        ChatAccount::SignedIn { device_id, .. } => device_id.clone(),
        other => panic!("等设备密钥前应已登录：{other:?}"),
    };
    let owner: OwnedUserId = OWNER.parse().unwrap();
    let deadline = tokio::time::Instant::now() + WAIT;
    let mut forwarded = home.key_uploads_forwarded();
    loop {
        if bot
            .server_device_ids_of(&owner)
            .await
            .expect("查主人的设备")
            .iter()
            .any(|id| id.as_str() == device_id)
        {
            return;
        }
        // 等下一次 keys/upload 被放行（计数变化）。放行到 Tuwunel 入库之间有几毫秒差，放行后
        // 短轮询 3 秒吸收这个延迟——事件仍是触发器，查不到才要求再放行一次。总时限是 WAIT。
        if tokio::time::timeout_at(deadline, forwarded.changed())
            .await
            .is_err()
        {
            panic!(
                "等主人的设备密钥（{device_id}）上服务器超时：手机的 keys/upload 没有被放行或放了还是查不到（放行计数 {}）",
                *forwarded.borrow()
            );
        }
        let settle = tokio::time::Instant::now() + Duration::from_secs(3);
        while tokio::time::Instant::now() < settle {
            if bot
                .server_device_ids_of(&owner)
                .await
                .expect("查主人的设备")
                .iter()
                .any(|id| id.as_str() == device_id)
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
    }
}

/// Bot 发来加密私聊的邀请，等房间名同步成 Bot 的名字、对方也认出来后主人再接受。
async fn accept_the_bots_invite(home: &Home, phone: &Phone, bot: &MatrixSession) -> OwnedRoomId {
    wait_for_owner_device_keys(home, phone, bot).await;
    let owner: OwnedUserId = OWNER.parse().unwrap();
    let room_id = bot
        .create_direct_chat(&owner, true)
        .await
        .expect("Bot 发起加密私聊");
    let invite = phone
        .wait_chat("收到 Bot 的邀请", |view| {
            view.rooms
                .iter()
                .find(|row| {
                    row.room_id == room_id.as_str()
                        && row.membership == RoomMembership::Invited
                        && row.is_direct
                        && row.name == "Hermes"
                        && row.direct_user_id.as_deref() == Some(bot.user_id().as_str())
                })
                .cloned()
        })
        .await;
    assert_eq!(invite.name, "Hermes");
    assert_eq!(
        invite.direct_user_id.as_deref(),
        Some(bot.user_id().as_str())
    );
    phone
        .dispatch(Action::JoinRoom {
            room_id: room_id.to_string(),
        })
        .unwrap();
    let joined = phone
        .wait_chat("加入私聊", |view| {
            view.rooms
                .iter()
                .find(|row| {
                    row.room_id == room_id.as_str()
                        && row.membership == RoomMembership::Joined
                        && row.is_direct
                        && row.direct_user_id.as_deref() == Some(bot.user_id().as_str())
                })
                .cloned()
        })
        .await;
    assert!(joined.is_encrypted);
    assert_eq!(
        joined.direct_user_id.as_deref(),
        Some(bot.user_id().as_str())
    );
    room_id
}

/// 收发文字和图片：时间线按增量更新，图片凭句柄取到解密后的原图。
async fn chat_with_the_bot(phone: &Phone, bot: &MatrixSession, room_id: &OwnedRoomId) {
    let bot_room = bot.open_room(room_id).await.expect("Bot 打开私聊");
    bot_room.send_text("你好，主人").await.expect("Bot 发文字");
    let mut timeline = Timeline::open(phone, room_id.as_str()).await;
    timeline
        .until(
            "收到 Bot 的文字，发送者显示 Bot 的名字",
            |item| sender(item, "你好，主人").filter(|name| name == "Hermes"),
        )
        .await;

    phone
        .dispatch(Action::SendText {
            room_id: room_id.to_string(),
            body: "  你好，Hermes  ".to_owned(),
        })
        .unwrap();
    bot_receives(&bot_room, "Bot 收到主人的文字", |message| {
        (message.body == MessageBody::Text("你好，Hermes".to_owned())).then_some(())
    })
    .await;

    // 加密房间里服务器做不了缩略图，图片都由发送端带上缩略图。
    let upload = wonton_matrix::ImageUpload {
        filename: "dot.png".to_owned(),
        mime_type: "image/png".to_owned(),
        data: PNG.to_vec(),
        width: Some(2),
        height: Some(2),
        thumbnail: Some(wonton_matrix::ImageThumbnail {
            mime_type: "image/png".to_owned(),
            data: THUMBNAIL.to_vec(),
            width: 1,
            height: 1,
        }),
    };
    bot_room.send_image(upload).await.expect("Bot 发图片");
    let image = timeline.until("收到 Bot 的图片", bots_image).await;
    let bytes = phone.load_media(image.media).await.expect("取图片");
    assert_eq!(bytes, PNG, "取到的图片和原图不一致");
    let thumbnail = image.thumbnail.expect("Bot 的图片带缩略图");
    let bytes = phone.load_media(thumbnail).await.expect("取缩略图");
    assert_eq!(bytes, THUMBNAIL, "取到的缩略图和发送的不一致");

    // 主人发图片：核心上传原图和壳生成的缩略图，时间线里出现自己的图片，Bot 收到的也带缩略图。
    phone
        .dispatch(Action::SendImage {
            room_id: room_id.to_string(),
            image: ImageUpload {
                filename: "reply.png".to_owned(),
                mime_type: "image/png".to_owned(),
                data: PNG.to_vec(),
                width: Some(2),
                height: Some(2),
                thumbnail: Some(ImageThumbnail {
                    mime_type: "image/png".to_owned(),
                    data: THUMBNAIL.to_vec(),
                    width: 1,
                    height: 1,
                }),
            },
        })
        .unwrap();
    timeline
        .until("自己的图片已发出", |item| match &item.content {
            TimelineContent::Message { message }
                if message.is_own
                    && message.event_id.is_some()
                    && matches!(&message.body, MessageContent::Image { image } if image.filename == "reply.png") =>
            {
                Some(())
            }
            _ => None,
        })
        .await;
    let received = bot_receives(&bot_room, "Bot 收到主人的图片", |message| match &message
        .body
    {
        MessageBody::Image(image) if image.filename == "reply.png" => image.thumbnail_token(),
        _ => None,
    })
    .await;
    let bytes = bot.download_media(&received).await.expect("Bot 取缩略图");
    assert_eq!(bytes, THUMBNAIL, "Bot 取到的缩略图和主人发的不一致");
}

/// 等 Bot 的时间线里出现满足条件的消息。
async fn bot_receives<R>(
    room: &ChatRoom,
    what: &str,
    mut check: impl FnMut(&Message) -> Option<R>,
) -> R {
    eventually(what, room.message_updates().await, |messages| {
        messages.iter().find_map(&mut check)
    })
    .await
}

/// 标为已读后未读清零；等待自动开启恢复并得到恢复密钥。
async fn read_and_wait_for_recovery(phone: &Phone, room_id: &OwnedRoomId) -> SecretString {
    phone
        .dispatch(Action::MarkRoomRead {
            room_id: room_id.to_string(),
        })
        .unwrap();
    phone
        .wait_chat("未读清零", |view| {
            view.rooms
                .iter()
                .any(|row| row.room_id == room_id.as_str() && row.unread_count == 0)
                .then_some(())
        })
        .await;
    phone
        .wait_chat("恢复自动开启", |view| {
            (view.encryption.recovery == RecoveryStatus::Enabled)
                .then(|| view.encryption.recovery_key.clone())
                .flatten()
        })
        .await
}

fn text(item: &TimelineItem, expected: &str) -> Option<()> {
    match &item.content {
        TimelineContent::Message { message } => {
            matches!(&message.body, MessageContent::Text { body, .. } if body == expected)
                .then_some(())
        }
        _ => None,
    }
}

/// 正文是 `expected` 的文字消息的发送者显示名。
fn sender(item: &TimelineItem, expected: &str) -> Option<String> {
    match &item.content {
        TimelineContent::Message { message } => {
            text(item, expected).map(|()| message.sender_name.clone())
        }
        _ => None,
    }
}

fn bots_image(item: &TimelineItem) -> Option<ImageContent> {
    match &item.content {
        TimelineContent::Message { message } if !message.is_own => match &message.body {
            MessageContent::Image { image } => Some(image.clone()),
            _ => None,
        },
        _ => None,
    }
}
