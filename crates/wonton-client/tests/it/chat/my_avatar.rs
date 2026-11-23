//! 换自己的头像（#662）：文件传进媒体库、`mxc://` 写进资料，自己的 `me()` 和别的设备看到的消息头像都跟着变；
//! 清掉以后两边都没有。家人和主人走同一条路（都用会话自己的资料），这里用同一账号的两台设备验证。

use wonton_client::{Action, MessageContent, TimelineContent};
use wonton_protocol::{
    AvatarKind, AvatarMediaProblem, AvatarUpload, MyAvatarProblem, TimelineAction, TimelineTarget,
};

use super::PNG;
use super::accept_the_bots_invite;
use super::support::{Home, Phone, Timeline};

fn upload(data: &[u8]) -> AvatarUpload {
    AvatarUpload {
        mime_type: "image/png".to_owned(),
        data: data.to_vec(),
        poster: None,
    }
}

fn send(phone: &Phone, room: &str, body: &str) {
    phone
        .dispatch(Action::Timeline {
            action: TimelineAction::SendText {
                target: TimelineTarget {
                    room_id: room.to_owned(),
                    thread_root: None,
                },
                body: body.to_owned(),
                reply_to: None,
                mentions: vec![],
            },
        })
        .unwrap();
}

/// 等正文是 `body` 的消息出来，并且发送者头像是 `expected`：成员资料可能比消息晚一点同步到。
async fn wait_avatar(
    timeline: &mut Timeline,
    body: &str,
    expected: Option<&str>,
) -> Option<String> {
    timeline
        .until("消息头像变了", |item| match &item.content {
            TimelineContent::Message { message }
                if matches!(&message.body, MessageContent::Text { body: text, .. } if text == body) =>
            {
                let actual = message.sender_avatar.as_ref().map(|avatar| avatar.token.clone());
                (actual.as_deref() == expected).then_some(actual)
            }
            _ => None,
        })
        .await
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn the_new_avatar_reaches_the_other_device_and_clear_one_takes_it_away() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let second_dir = tempfile::tempdir().unwrap();
    let second = Phone::open(second_dir.path());
    let home = Home::start(&[phone.device_id(), second.device_id()]).await;
    let bot = home.bot("hermes", "Hermes").await;
    phone.connect(&home);
    phone.signed_in().await;
    second.connect(&home);
    second.signed_in().await;
    let room = accept_the_bots_invite(&home, &phone, &bot).await;
    let room = room.as_str();

    // 太大的文件不去占服务器，直接说文件不行。
    let too_big = upload(&vec![0; wonton_protocol::MAX_AVATAR_BYTES as usize + 1]);
    assert_eq!(
        phone.set_my_avatar(too_big).await.unwrap_err(),
        MyAvatarProblem::MediaRejected {
            problem: AvatarMediaProblem::TooLarge
        }
    );

    let me = phone.set_my_avatar(upload(PNG)).await.expect("换头像");
    let avatar = me.avatar.expect("换完的 me 带新头像");
    assert_eq!(avatar.kind, AvatarKind::Still);
    assert_eq!(
        phone.me().await.expect("读回 me").avatar,
        Some(avatar.clone())
    );

    let mut own = Timeline::open(&phone, room).await;
    let mut other = Timeline::open(&second, room).await;
    send(&phone, room, "换了头像");
    assert_eq!(
        wait_avatar(&mut own, "换了头像", Some(avatar.media.token.as_str())).await,
        Some(avatar.media.token.clone())
    );
    assert_eq!(
        wait_avatar(&mut other, "换了头像", Some(avatar.media.token.as_str())).await,
        Some(avatar.media.token.clone())
    );

    let me = phone.clear_my_avatar().await.expect("清头像");
    assert_eq!(me.avatar, None);
    assert_eq!(phone.me().await.expect("再读回 me").avatar, None);
    // 清两次也一样。
    phone.clear_my_avatar().await.expect("再清一次");

    send(&phone, room, "清掉头像了");
    assert_eq!(wait_avatar(&mut own, "清掉头像了", None).await, None);
    assert_eq!(wait_avatar(&mut other, "清掉头像了", None).await, None);
}
