//! Agent 一轮任务的进度（ADR-0082）：Bot 发的状态消息在时间线里是一条随编辑推进的 AgentRun；只对 Bot 账号
//! 解读；Bot 离线时显示成不确定；waiting 时带上这个 Bot 等回答的核心卡片；Bot 自己发的 `choices` 以反应回答。

use std::time::Duration;

use serde_json::{Value, json};

use wonton_client::{Action, Answer, MessageContent, TimelineContent};
use wonton_matrix::{ChatRoom, MatrixSession};
use wonton_protocol::{AgentRunChoice, AgentRunState, BotActivityFacts};

use super::confirmation::{card, content, core_room, in_ten_minutes, joined_direct_chat, send};
use super::support::{Home, Phone, Timeline};
use crate::plugins::rig::running;

const PLUGIN: &str = "app.bot";

/// Bot 发的一条状态消息（MSC4564 的格式，ADR-0082）。桥现在发 `org.fluxchat.agent`。
fn status(state: &str, extra: Value) -> Value {
    let mut object = json!({"v": 1, "state": state});
    if let (Some(object), Some(extra)) = (object.as_object_mut(), extra.as_object()) {
        for (key, value) in extra {
            object.insert(key.clone(), value.clone());
        }
    }
    json!({
        "msgtype": "m.notice",
        "body": format!("状态是 {state}"),
        "org.fluxchat.agent": object,
    })
}

async fn edit_status(room: &ChatRoom, event: &str, state: &str, extra: Value) {
    let new_content = status(state, extra);
    let body = new_content["body"].as_str().unwrap().to_owned();
    let mut edit = new_content.clone();
    edit["body"] = json!(body);
    edit["m.new_content"] = new_content;
    edit["m.relates_to"] = json!({"rel_type": "m.replace", "event_id": event});
    send(room, edit).await;
}

/// 时间线里的一条 AgentRun 条目的状态拷贝；夹具可以不带 `run`，这里不拿它当匹配条件。
fn agent_run(item: &TimelineContent) -> Option<AgentRunState> {
    match item {
        TimelineContent::AgentRun { state, .. } => Some(state.clone()),
        _ => None,
    }
}

struct Setup {
    phone: Phone,
    _home: Home,
    hermes: MatrixSession,
    room: wonton_matrix::OwnedRoomId,
    timeline: Timeline,
    bot_room: ChatRoom,
}

async fn setup() -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    phone.signed_in().await;
    let hermes = home.bot("hermes", "Hermes").await;
    let mut installed = running(PLUGIN);
    installed.bot_user_id = Some(hermes.user_id().to_string());
    home.set_installed_plugins(vec![installed]);
    // 状态行按 Bot 的活动合成：先给一份“在运行”的报告，等 Bot 列表带上活动，之后的断言才不会撞上不确定。
    home.bot_api.facts(BotActivityFacts {
        plugin_id: PLUGIN.to_owned(),
        bot: Some("default".into()),
        user_id: hermes.user_id().to_string(),
        running: true,
        model_requests: 0,
        tool_calls: 0,
        waiting: 0,
        profile_rev: 0,
    });
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
        _home: home,
        hermes,
        room,
        timeline,
        bot_room,
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_round_with_edits_is_one_agent_run_following_the_latest_edit() {
    let mut setup = setup().await;
    let (phone, room, timeline, bot_room) = (
        &setup.phone,
        &setup.room,
        &mut setup.timeline,
        &setup.bot_room,
    );

    // 主人发请求，Bot 对它发一条状态消息，再编辑两次推进到完成。
    phone
        .dispatch(Action::SendText {
            room_id: room.to_string(),
            body: "帮我查天气".to_owned(),
            reply_to: None,
        })
        .unwrap();
    let request: String = timeline
        .until("请求发出去了", |item| match &item.content {
            TimelineContent::Message { message }
                if message.is_own
                    && matches!(
                        &message.body,
                        MessageContent::Text { body, .. } if body == "帮我查天气"
                    ) =>
            {
                message.event_id.clone()
            }
            _ => None,
        })
        .await;

    bot_room
        .send_raw_message(status(
            "working",
            json!({"run": request, "step": "在想", "steps": 1}),
        ))
        .await
        .unwrap();
    let entry = timeline
        .until("状态行出现了", |item| {
            agent_run(&item.content)
                .and_then(|state| (state == AgentRunState::Working).then(|| item.id.clone()))
        })
        .await;

    edit_status(
        bot_room,
        &entry,
        "working",
        json!({"run": request, "step": "执行命令", "steps": 2}),
    )
    .await;
    edit_status(
        bot_room,
        &entry,
        "done",
        json!({"run": request, "steps": 2, "result": "查到了"}),
    )
    .await;

    // 发 + 两次编辑在时间线里只是一条 AgentRun，状态跟着最新编辑走。
    let (runs, done) = timeline
        .until_items("一条 AgentRun 推进到完成", |items| {
            let runs = items
                .iter()
                .filter(|item| matches!(&item.content, TimelineContent::AgentRun { .. }))
                .count();
            let done = items.iter().find_map(|item| match &item.content {
                TimelineContent::AgentRun {
                    run, state, steps, ..
                } if run.as_deref() == Some(request.as_str())
                    && *state == AgentRunState::Done
                    && *steps == Some(2) =>
                {
                    Some(item.id.clone())
                }
                _ => None,
            })?;
            Some((runs, done))
        })
        .await;
    assert_eq!(runs, 1, "一轮在时间线里只有一条 AgentRun");
    assert_eq!(done, entry, "编辑换的是同一条");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn unknown_states_fall_back_and_v2_stays_a_plain_message() {
    let mut setup = setup().await;
    let (timeline, bot_room) = (&mut setup.timeline, &setup.bot_room);

    bot_room
        .send_raw_message(status("brewing", json!({})))
        .await
        .expect("Bot 发消息");
    timeline
        .until("未知状态落到 Other", |item| {
            agent_run(&item.content).and_then(|state| {
                matches!(&state, AgentRunState::Other(raw) if raw == "brewing").then_some(())
            })
        })
        .await;

    bot_room
        .send_raw_message(json!({
            "msgtype": "m.notice",
            "body": "版本太新",
            "org.fluxchat.agent": {"v": 2, "state": "working"},
        }))
        .await
        .expect("Bot 发消息");
    timeline
        .until("v 不是 1 按普通消息", |item| match &item.content {
            TimelineContent::Message { message }
                if matches!(&message.body,
                MessageContent::Text { body, .. } if body == "版本太新") =>
            {
                Some(())
            }
            _ => None,
        })
        .await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_non_bot_sender_stays_a_plain_message() {
    let mut setup = setup().await;
    let (home, room, timeline, bot_room) = (
        &setup._home,
        &setup.room,
        &mut setup.timeline,
        &setup.bot_room,
    );
    let other = home.bot("colleague", "同事").await;
    bot_room.invite(other.user_id()).await.expect("邀请同事");
    other.join_room(room).await.expect("同事加入");
    let other_room = other.open_room(room).await.expect("同事打开房间");

    other_room
        .send_raw_message(status("working", json!({})))
        .await
        .expect("Bot 发消息");
    timeline
        .until("不是 Bot 发的按普通消息", |item| {
            match &item.content {
                TimelineContent::Message { message }
                    if matches!(&message.body,
                MessageContent::Text { body, .. } if body == "状态是 working") =>
                {
                    Some(())
                }
                _ => None,
            }
        })
        .await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn an_offline_bot_shows_uncertain_until_it_is_back() {
    let mut setup = setup().await;
    let (home, hermes, timeline, bot_room) = (
        &setup._home,
        &setup.hermes,
        &mut setup.timeline,
        &setup.bot_room,
    );
    let mut facts = BotActivityFacts {
        plugin_id: PLUGIN.to_owned(),
        bot: Some("default".into()),
        user_id: hermes.user_id().to_string(),
        running: false,
        model_requests: 0,
        tool_calls: 0,
        waiting: 0,
        profile_rev: 0,
    };
    home.bot_api.facts(facts.clone());

    bot_room
        .send_raw_message(status("working", json!({})))
        .await
        .expect("Bot 发消息");
    timeline
        .until("离线的 Bot 是不确定", |item| {
            agent_run(&item.content)
                .and_then(|state| (state == AgentRunState::Uncertain).then_some(()))
        })
        .await;

    facts.running = true;
    home.bot_api.facts(facts);
    timeline
        .until("回到在线恢复 working", |item| {
            agent_run(&item.content)
                .and_then(|state| (state == AgentRunState::Working).then_some(()))
        })
        .await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn waiting_carries_the_pending_card_until_it_is_answered() {
    let mut setup = setup().await;
    let (phone, home, hermes, room, timeline, bot_room) = (
        &setup.phone,
        &setup._home,
        &setup.hermes,
        &setup.room,
        &mut setup.timeline,
        &setup.bot_room,
    );

    // 核心在这个房间里发一张这个 Bot 发起的卡片，等主人回答。核心得先进房间才能发。
    let (core, _notices) = core_room(phone, home).await;
    bot_room.invite(core.user_id()).await.expect("邀请核心");
    core.join_room(room).await.expect("核心加入");
    let core_room = core.open_room(room).await.expect("核心打开房间");
    let mut pending = card("card-1", in_ten_minutes());
    pending["requester"] = json!({
        "type": "bot",
        "bot": {"plugin_id": PLUGIN, "bot": "default"},
        "user_id": hermes.user_id().to_string(),
    });
    send(&core_room, content(&pending)).await;
    phone
        .wait_chat("卡片在等回答", |view| {
            view.pending_confirmations
                .iter()
                .any(|card| card.id == "card-1")
                .then_some(())
        })
        .await;

    bot_room
        .send_raw_message(status("waiting", json!({})))
        .await
        .expect("Bot 发消息");
    let card_id = timeline
        .until("waiting 带上卡片", |item| match &item.content {
            TimelineContent::AgentRun { state, card_id, .. }
                if *state == AgentRunState::Waiting =>
            {
                card_id.clone()
            }
            _ => None,
        })
        .await;
    assert_eq!(card_id, "card-1");

    phone
        .dispatch(Action::AnswerConfirmation {
            id: "card-1".to_owned(),
            answer: Answer::Reject,
            choice_id: None,
            secrets: Default::default(),
            settings: Default::default(),
        })
        .unwrap();
    phone
        .wait_chat("卡片回答过了", |view| {
            (!view
                .pending_confirmations
                .iter()
                .any(|card| card.id == "card-1"))
            .then_some(())
        })
        .await;
    timeline
        .until("卡片结束后不带 ID", |item| match &item.content {
            TimelineContent::AgentRun { card_id, .. } => (card_id.is_none()).then_some(()),
            _ => None,
        })
        .await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn choices_answer_with_a_reaction_only_when_tapped() {
    let mut setup = setup().await;
    let (phone, hermes, room, timeline, bot_room) = (
        &setup.phone,
        &setup.hermes,
        &setup.room,
        &mut setup.timeline,
        &setup.bot_room,
    );

    bot_room
        .send_raw_message(status(
            "waiting",
            json!({"choices": [{"key": "allow", "label": "继续"}]}),
        ))
        .await
        .expect("Bot 发消息");
    let (entry, choices) = timeline
        .until("选项带出来了", |item| match &item.content {
            TimelineContent::AgentRun { state, choices, .. }
                if *state == AgentRunState::Waiting && !choices.is_empty() =>
            {
                Some((item.id.clone(), choices.clone()))
            }
            _ => None,
        })
        .await;
    assert_eq!(
        choices,
        vec![AgentRunChoice {
            key: "allow".into(),
            label: "继续".into()
        }]
    );

    // 还没点不发。反应是对状态消息的 `m.annotation`，聚合在状态事件的 unsigned 里；从 Bot 那边读回来核对。
    let status_event: wonton_matrix::OwnedEventId = entry.parse().unwrap();
    let annotation_of_status = || async {
        let event = hermes
            .client()
            .get_room(room)
            .expect("Bot 在房间里")
            .load_or_fetch_event(&status_event, None)
            .await
            .expect("状态消息取得到");
        let raw = event.raw().json().get();
        let chunk =
            serde_json::from_str::<Value>(raw).unwrap()["unsigned"]["m.relations"]["m.annotation"]
                ["chunk"]
                .as_array()
                .cloned()
                .unwrap_or_default();
        chunk.iter().any(|reaction| {
            reaction["sender"] == json!(super::support::OWNER)
                && reaction["content"]["m.relates_to"]["event_id"] == json!(status_event)
                && reaction["content"]["m.relates_to"]["key"] == "allow"
        })
    };
    assert!(!annotation_of_status().await, "没点不发反应");

    phone
        .dispatch(Action::AnswerAgentChoice {
            room_id: room.to_string(),
            item_id: entry,
            key: "allow".to_owned(),
        })
        .unwrap();
    // 点了：Bot 那边看到主人发的反应。
    let waiting = async {
        loop {
            if annotation_of_status().await {
                break;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    };
    tokio::time::timeout(super::support::WAIT, waiting)
        .await
        .expect("反应发出来了");
}
