//! 消息带着发送者资料里的头像：没设头像的发送者为 `None`，自己的消息也照填（壳按需画）。

use wonton_client::{Action, MessageContent, TimelineContent};
use wonton_protocol::{TimelineAction, TimelineTarget};

use super::confirmation::joined_direct_chat;
use super::support::{Home, Phone, Timeline};

const AVATAR: &str = "mxc://example.org/hermes-avatar";

fn avatar_of<'a>(timeline: &'a Timeline, body: &str) -> Option<&'a str> {
    let avatar = timeline.items.iter().find_map(|item| match &item.content {
        TimelineContent::Message { message }
            if matches!(&message.body, MessageContent::Text { body: text, .. } if text == body) =>
        {
            Some(message.sender_avatar.as_ref())
        }
        _ => None,
    });
    avatar?.map(|avatar| avatar.token.as_str())
}

/// 等到正文是 `body` 的消息出现（不看有没有头像）。
async fn wait_message(timeline: &mut Timeline, body: &str) {
    timeline
        .until("消息出现", |item| match &item.content {
            TimelineContent::Message { message }
                if matches!(&message.body, MessageContent::Text { body: text, .. } if text == body) =>
            {
                Some(())
            }
            _ => None,
        })
        .await;
}

/// 等到正文是 `body` 的消息带上发送者头像：成员资料可能比消息晚到，没头像时继续等。
async fn wait_avatar(timeline: &mut Timeline, body: &str) -> String {
    timeline
        .until("消息带发送者头像", |item| match &item.content {
            TimelineContent::Message { message }
                if matches!(&message.body, MessageContent::Text { body: text, .. } if text == body) =>
            {
                message.sender_avatar.as_ref().map(|avatar| avatar.token.clone())
            }
            _ => None,
        })
        .await
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn the_senders_avatar_travels_with_the_message() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    phone.signed_in().await;
    let hermes = home.bot("hermes", "Hermes").await;
    let avatar: wonton_matrix::OwnedMxcUri = AVATAR.into();
    hermes
        .client()
        .account()
        .set_avatar_url(Some(&avatar))
        .await
        .expect("Bot 设置头像");
    let room = joined_direct_chat(&phone, &hermes).await;
    let mut timeline = Timeline::open(&phone, room.as_str()).await;
    hermes
        .send_text_to(&room, "Bot 发的")
        .await
        .expect("Bot 发消息");
    phone
        .dispatch(Action::Timeline {
            action: TimelineAction::SendText {
                target: TimelineTarget {
                    room_id: room.to_string(),
                    thread_root: None,
                },
                body: "主人发的".to_owned(),
                reply_to: None,
                mentions: vec![],
            },
        })
        .unwrap();
    wait_message(&mut timeline, "主人发的").await;
    wait_message(&mut timeline, "Bot 发的").await;
    let bot_avatar = wait_avatar(&mut timeline, "Bot 发的").await;
    let own_avatar = avatar_of(&timeline, "主人发的");
    let token = wonton_matrix::media_token(AVATAR).expect("头像句柄");
    assert_eq!(bot_avatar, token);
    assert_eq!(own_avatar, None);
    assert_eq!(avatar_of(&timeline, "Bot 发的"), Some(token.as_str()));
}
