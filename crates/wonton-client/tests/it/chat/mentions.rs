//! @ 提及（ADR-0109 第 6 节）：群里发送带选中的 `m.mentions`，回复把被回复的人也算上；私聊不给候选。

use wonton_client::{Action, MessageContent, TimelineContent};
use wonton_matrix::{Message, MessageBody};
use wonton_protocol::BotChatAvailability;
use wonton_protocol::{
    BotActivityFacts, ComposerViewModel, TimelineAction, TimelineMessage, TimelineTarget,
};

use super::message_actions::{invite_to_group_room, join};
use super::support::{Home, Phone, Timeline, WAIT};
use super::{accept_the_bots_invite, bot_receives};

fn target(room_id: &str) -> TimelineTarget {
    TimelineTarget {
        room_id: room_id.to_owned(),
        thread_root: None,
    }
}

fn send(room_id: &str, body: &str, reply_to: Option<String>, mentions: Vec<String>) -> Action {
    Action::Timeline {
        action: TimelineAction::SendText {
            target: target(room_id),
            body: body.to_owned(),
            reply_to,
            mentions,
        },
    }
}

async fn wait_candidates(
    phone: &Phone,
    room: &str,
    when: impl Fn(&ComposerViewModel) -> bool,
) -> ComposerViewModel {
    let subscription = phone.subscribe_composer(target(room)).unwrap();
    tokio::time::timeout(WAIT, async {
        loop {
            let view = subscription.next().await.expect("输入框订阅仍有效");
            if when(&view) {
                return view;
            }
        }
    })
    .await
    .expect("等输入框的 @ 候选")
}

fn text(message: &Message) -> String {
    match &message.body {
        MessageBody::Text(body) => body.clone(),
        other => panic!("不是文字：{other:?}"),
    }
}

fn item_text(message: &TimelineMessage) -> String {
    match &message.body {
        MessageContent::Text { body, .. } => body.clone(),
        other => panic!("不是文字：{other:?}"),
    }
}

/// 群里有 Bot 和另一个人：候选是这两个，Bot 在前；发送只在选中时写 `m.mentions`，回复把对方也带上。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn group_sends_selected_mentions_and_reply_adds_the_quoted_user() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    phone.signed_in().await;
    let hermes = home.bot("hermes", "Hermes").await;
    let tom = home.bot("tom", "Tom").await;
    let room_id = invite_to_group_room(&phone, &hermes, "三个人的群").await;
    join(&phone, &room_id).await;
    hermes
        .invite_user(&room_id, tom.user_id())
        .await
        .expect("邀请 Tom");
    tom.join_room(&room_id).await.expect("Tom 加入");
    let hermes_room = hermes.open_room(&room_id).await.expect("Bot 打开群聊");
    home.bot_api.bots(vec![BotActivityFacts {
        plugin_id: "app.hermes".into(),
        bot: Some("default".into()),
        user_id: hermes.user_id().to_string(),
        chat: BotChatAvailability::Available,
        running: true,
        model_requests: 0,
        tool_calls: 0,
        waiting: 0,
        profile_rev: 0,
    }]);
    let room = room_id.to_string();

    let view = wait_candidates(&phone, &room, |view| view.mention_candidates.len() == 2).await;
    assert_eq!(
        view.mention_candidates[0].user_id,
        hermes.user_id().as_str()
    );
    assert!(view.mention_candidates[0].is_bot);
    assert_eq!(view.mention_candidates[1].user_id, tom.user_id().as_str());
    assert!(!view.mention_candidates[1].is_bot);

    phone
        .dispatch(send(
            &room,
            "@Hermes你好",
            None,
            vec![hermes.user_id().to_string()],
        ))
        .unwrap();
    let mentioned = bot_receives(&hermes_room, "收到提到 Hermes 的消息", |message| {
        (text(message) == "@Hermes你好").then(|| message.clone())
    })
    .await;
    assert_eq!(mentioned.mentions, vec![hermes.user_id().to_owned()]);

    phone
        .dispatch(send(&room, "没点名", None, Vec::new()))
        .unwrap();
    let plain = bot_receives(&hermes_room, "收到没点名的消息", |message| {
        (text(message) == "没点名").then(|| message.clone())
    })
    .await;
    assert!(plain.mentions.is_empty());

    hermes_room
        .send_text("要回的话", &[])
        .await
        .expect("Bot 发原话");
    let mut timeline = Timeline::open(&phone, &room).await;
    let (_, quoted) = timeline
        .until("收到要回的话", |item| match &item.content {
            TimelineContent::Message { message } => {
                (item_text(message) == "要回的话").then(|| (item.id.clone(), message.clone()))
            }
            _ => None,
        })
        .await;
    phone
        .dispatch(send(
            &room,
            "回一句",
            Some(quoted.event_id.unwrap().to_string()),
            Vec::new(),
        ))
        .unwrap();
    let reply = bot_receives(&hermes_room, "收到回复", |message| {
        (text(message) == "回一句").then(|| message.clone())
    })
    .await;
    assert_eq!(reply.mentions, vec![hermes.user_id().to_owned()]);
}

/// 两个人的私聊里人就是对方，不给候选。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn direct_room_has_no_candidates() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    phone.signed_in().await;
    let bot = home.bot("hermes", "Hermes").await;
    let room_id = accept_the_bots_invite(&home, &phone, &bot).await;
    let _bot_room = bot.open_room(&room_id).await.expect("Bot 打开私聊");
    let view = wait_candidates(&phone, room_id.as_str(), |_| true).await;
    assert!(view.mention_candidates.is_empty());
}
