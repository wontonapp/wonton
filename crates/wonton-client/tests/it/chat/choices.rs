//! 消息下面的选项按钮（ADR-0109 第 8 节）：发送者自己先放的表情是选项，点它按主人的身份加同一个表情；
//! 发失败停在按钮上，点它重发，不重复。

use serde_json::{Value, json};

use wonton_client::{Action, TimelineContent, TimelineItem};
use wonton_matrix::testkit::eventually;
use wonton_matrix::{ChatRoom, EventId, MatrixSession, OwnedEventId, OwnedRoomId, RelationType};
use wonton_protocol::BotChatAvailability;
use wonton_protocol::{BotActivityFacts, ChatAction, ChoiceSend, ChoicesState, MessageChoices};

use super::confirmation::joined_direct_chat;
use super::support::{Home, Phone, Timeline};
use crate::plugins::rig::running;

const PLUGIN: &str = "app.bot";
const BODY: &str = "Fixture approval\n\n✅ = approve once\n❌ = deny";

fn options(item: &TimelineItem) -> Option<&MessageChoices> {
    match &item.content {
        TimelineContent::Message { message } => message.choices.as_ref(),
        _ => None,
    }
}

/// 等两个选项都可点、标签是正文里的说明：返回这条消息的条目 ID。
async fn approval(timeline: &mut Timeline) -> String {
    timeline
        .until("消息下面出现两个可点的选项", |item| {
            let choices = options(item)?;
            let labels = choices
                .options
                .iter()
                .map(|option| (option.key.as_str(), option.label.as_deref()))
                .collect::<Vec<_>>();
            (choices.state == ChoicesState::Open
                && labels == [("✅", Some("approve once")), ("❌", Some("deny"))])
            .then(|| item.id.clone())
        })
        .await
}

/// 等 Bot 那边的这条消息上出现主人加的这些反应键（不看顺序）。
async fn wait_owner_reactions(
    bot_room: &ChatRoom,
    event: &EventId,
    expected: &[&str],
) -> Vec<String> {
    let mut expected = expected
        .iter()
        .map(|key| (*key).to_owned())
        .collect::<Vec<_>>();
    expected.sort();
    eventually(
        "Bot 那边看到主人的表情",
        bot_room.message_updates().await,
        |messages| {
            let message = messages
                .iter()
                .find(|message| message.event_id.as_deref() == Some(event))?;
            let mut keys = message
                .reactions
                .iter()
                .filter(|reaction| {
                    reaction
                        .senders
                        .iter()
                        .any(|user| user.as_str() == super::support::OWNER)
                })
                .map(|reaction| reaction.key.clone())
                .collect::<Vec<_>>();
            keys.sort();
            (keys == expected).then_some(keys)
        },
    )
    .await
}

/// Bot 那边这条消息上主人加的反应键，按出现顺序。
async fn owner_reactions(
    hermes: &MatrixSession,
    room: &OwnedRoomId,
    event: &EventId,
) -> Vec<String> {
    let (_, relations) = hermes
        .client()
        .get_room(room)
        .expect("Bot 在房间里")
        .load_or_fetch_event_with_relations(event, Some(vec![RelationType::Annotation]), None)
        .await
        .expect("消息和它的反应取得到");
    relations
        .iter()
        .filter_map(|reaction| {
            let raw = serde_json::from_str::<Value>(reaction.raw().json().get()).unwrap();
            (raw["sender"] == json!(super::support::OWNER))
                .then(|| {
                    raw["content"]["m.relates_to"]["key"]
                        .as_str()
                        .map(str::to_owned)
                })
                .flatten()
        })
        .collect()
}

fn tap(phone: &Phone, room: &str, item: &str, key: &str) {
    phone
        .dispatch(Action::Chat {
            action: ChatAction::AnswerAgentChoice {
                room_id: room.to_owned(),
                item_id: item.to_owned(),
                key: key.to_owned(),
            },
        })
        .unwrap();
}

struct Setup {
    phone: Phone,
    home: Home,
    _dir: tempfile::TempDir,
    hermes: MatrixSession,
    room: OwnedRoomId,
    timeline: Timeline,
    bot_room: ChatRoom,
}

async fn setup() -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    let hermes = home.bot("hermes", "Hermes").await;
    let mut installed = running(PLUGIN);
    installed.bot_user_id = Some(hermes.user_id().to_string());
    installed.default_bot_visibility = Some(wonton_protocol::DefaultBotVisibility::Visible);
    // 插件表和活动报告在连接前喂：follow 在连接建立时读一次，连接后再补的不一定被看到。
    home.set_installed_plugins(vec![installed]);
    home.bot_api.facts(BotActivityFacts {
        plugin_id: PLUGIN.to_owned(),
        bot: Some("default".into()),
        user_id: hermes.user_id().to_string(),
        chat: BotChatAvailability::Available,
        running: true,
        model_requests: 0,
        tool_calls: 0,
        waiting: 0,
        profile_rev: 0,
    });
    phone.connect(&home);
    phone.signed_in().await;
    phone
        .wait_chat("Bot 列表里有 Hermes", |view| {
            view.bots
                .iter()
                .any(|bot| bot.user_id == *hermes.user_id() && bot.activity.is_some())
                .then_some(())
        })
        .await;
    let room = joined_direct_chat(&phone, &hermes).await;
    let timeline = Timeline::open(&phone, room.as_str()).await;
    let bot_room = hermes.open_room(&room).await.expect("Bot 打开房间");
    Setup {
        phone,
        home,
        _dir: dir,
        hermes,
        room,
        timeline,
        bot_room,
    }
}

/// Bot 发正文，再给正文里列的每个表情加自己的反应。
async fn ask(room: &ChatRoom) -> OwnedEventId {
    let event = room
        .send_raw_message(json!({"msgtype": "m.notice", "body": BODY}))
        .await
        .expect("Bot 发消息");
    for key in ["✅", "❌"] {
        room.send_raw_event(
            "m.reaction",
            json!({"m.relates_to": {
                "rel_type": "m.annotation",
                "event_id": &event,
                "key": key,
            }}),
        )
        .await
        .expect("Bot 加表情");
    }
    event
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_senders_emoji_is_a_choice_and_tapping_it_answers_with_a_reaction() {
    let mut setup = setup().await;
    let (phone, hermes, room, timeline, bot_room) = (
        &setup.phone,
        &setup.hermes,
        &setup.room,
        &mut setup.timeline,
        &setup.bot_room,
    );
    let event = ask(bot_room).await;
    let item = approval(timeline).await;

    // 没点不发。
    assert_eq!(
        owner_reactions(hermes, room, &event).await,
        Vec::<String>::new()
    );

    tap(phone, room.as_str(), &item, "✅");
    let choices = timeline
        .until(
            "点了以后算答过，表情也发出去了",
            |item| match options(item) {
                Some(choices)
                    if choices.state == ChoicesState::Answered
                        && choices
                            .options
                            .iter()
                            .all(|option| option.send == ChoiceSend::Idle) =>
                {
                    Some(choices.clone())
                }
                _ => None,
            },
        )
        .await;
    assert_eq!(
        choices
            .options
            .iter()
            .map(|option| (option.key.as_str(), &option.send))
            .collect::<Vec<_>>(),
        [("✅", &ChoiceSend::Idle), ("❌", &ChoiceSend::Idle)]
    );
    assert_eq!(
        wait_owner_reactions(bot_room, &event, &["✅"]).await,
        ["✅"]
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_failed_reaction_stays_tappable_and_the_retry_does_not_duplicate_it() {
    let mut setup = setup().await;
    let (phone, home, hermes, room, timeline, bot_room) = (
        &setup.phone,
        &setup.home,
        &setup.hermes,
        &setup.room,
        &mut setup.timeline,
        &setup.bot_room,
    );
    let event = ask(bot_room).await;
    let item = approval(timeline).await;

    // 服务器拒收：按钮停在失败上，原因换成人话，选项还是开着的。
    home.refuse_matrix("send/m.reaction", 400, "M_UNKNOWN");
    tap(phone, room.as_str(), &item, "✅");
    let choices = timeline
        .until("失败停在按钮上", |item| {
            let choices = options(item)?;
            choices
                .options
                .iter()
                .any(|option| {
                    option.key == "✅" && matches!(option.send, ChoiceSend::Failed { .. })
                })
                .then(|| choices.clone())
        })
        .await;
    let failed = || {
        let option = choices
            .options
            .iter()
            .find(|option| option.key == "✅")
            .expect("还是选项");
        match &option.send {
            ChoiceSend::Failed { message } => message.clone(),
            other => panic!("不是失败：{other:?}"),
        }
    };
    assert!(!failed().is_empty(), "原因要是人话");
    assert_eq!(choices.state, ChoicesState::Open);
    assert_eq!(
        owner_reactions(hermes, room, &event).await,
        Vec::<String>::new()
    );

    // 放行后点它重发：答上了，Bot 那边只有一条。
    home.clear_refusals();
    tap(phone, room.as_str(), &item, "✅");
    timeline
        .until("重发成功", |item| {
            (options(item)?.state == ChoicesState::Answered).then_some(())
        })
        .await;
    assert_eq!(
        wait_owner_reactions(bot_room, &event, &["✅"]).await,
        ["✅"]
    );
}
