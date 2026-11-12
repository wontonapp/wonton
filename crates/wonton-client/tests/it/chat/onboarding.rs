//! 引导的最后一步依赖真实聊天：主人在和默认 Agent 的私聊里发过第一条消息后，候选提问就不再给，
//! 发过消息的房间记进本机（ADR-0034）。

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use wonton_protocol::{TimelineAction, TimelineTarget};

use serde_json::json;
use wonton_client::{Action, MessageContent, TimelineContent};
use wonton_protocol::OnboardingViewModel;

use super::accept_the_bots_invite;
use super::support::{Home, Phone, Timeline};

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn sending_to_the_bot_hides_the_prompts_and_records_the_room() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    home.set_onboarding(json!({
        "step": "finished",
        "primary_agent": "hermes",
        "replaced_agent_asked": true,
        "agents": [{
            "id": "hermes",
            "title": "Hermes",
            "source": null,
            "status": {"state": "running", "health": "healthy"},
            "bot_user_id": "@hermes:wonton.localhost",
            "logins": [],
            "blocked": null
        }],
        "needs": [],
        "prompts": ["问我能做什么"],
        "catalog_problem": null
    }));
    phone.connect(&home);
    phone.signed_in().await;

    let bot = home.bot("hermes", "Hermes").await;
    assert_eq!(bot.user_id().as_str(), "@hermes:wonton.localhost");
    let room_id = accept_the_bots_invite(&home, &phone, &bot).await;

    // 走完了引导、默认 Agent 的私聊在列表里，主人的第一条消息还没发：输入框上方给候选提问。
    let prompts = phone
        .wait_onboarding_prompts("候选提问出现", |view| {
            (!view.chat_prompts.is_empty()).then(|| view.chat_prompts.clone())
        })
        .await;
    assert_eq!(prompts, ["问我能做什么"]);

    let room = room_id.to_string();
    let mut timeline = Timeline::open(&phone, &room).await;
    phone
        .dispatch(Action::Timeline {
            action: TimelineAction::SendText {
                target: TimelineTarget {
                    room_id: room.clone(),
                    thread_root: None,
                },
                body: "你好".to_owned(),
                reply_to: None,
            },
        })
        .unwrap();
    timeline
        .until("消息已发出", |item| match &item.content {
            TimelineContent::Message { message } => match &message.body {
                MessageContent::Text { body, .. } if body == "你好" => Some(()),
                _ => None,
            },
            _ => None,
        })
        .await;

    // 发过消息：提问收起来，本机记下这个房间。
    phone
        .wait_onboarding_prompts("发过消息后提问收起", |view| {
            view.chat_prompts.is_empty().then_some(())
        })
        .await;
    let records = std::fs::read_to_string(dir.path().join("servers/1/onboarding.json")).unwrap();
    assert!(
        records.contains(&room),
        "发过消息的房间要记进本机：{records}"
    );
    assert!(records.contains("\"prompts_used\":true"));
}

/// 房间记进“发过消息”之后，任何一次发布出去之前 `prompts_used` 都必须已经在盘上：
/// 视图这边靠“发过消息的房间”就收起提问，盘上没写上，App 杀在这之间重启后还会再给一次提问。
/// rooms 列表自己会抖动，所以只看已经把房间记成发过消息的那些发布。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn prompts_used_reaches_the_disk_before_the_room_is_published_as_sent() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    home.set_onboarding(json!({
        "step": "finished",
        "primary_agent": "hermes",
        "replaced_agent_asked": true,
        "agents": [{
            "id": "hermes",
            "title": "Hermes",
            "source": null,
            "status": {"state": "running", "health": "healthy"},
            "bot_user_id": "@hermes:wonton.localhost",
            "logins": [],
            "blocked": null
        }],
        "needs": [],
        "prompts": ["问我能做什么"],
        "catalog_problem": null
    }));
    phone.connect(&home);
    phone.signed_in().await;

    let bot = home.bot("hermes", "Hermes").await;
    let room = accept_the_bots_invite(&home, &phone, &bot)
        .await
        .to_string();
    phone
        .wait_onboarding_prompts("候选提问出现", |view| {
            (!view.chat_prompts.is_empty()).then_some(())
        })
        .await;

    // 知道房间之后才设钩子：只记“已经把这个房间记成发过消息”的发布，回调在发布用的那把锁里跑。
    let file = dir.path().join("servers/1/onboarding.json");
    let watched = room.clone();
    let watched_file = file.clone();
    let seen: Arc<Mutex<Vec<(bool, bool)>>> = Arc::new(Mutex::new(Vec::new()));
    let recorder = Arc::clone(&seen);
    phone.test_set_onboarding_publish_hook(Some(Arc::new(
        move |view: &OnboardingViewModel, sent: &HashSet<String>| {
            if !sent.contains(&watched) {
                return;
            }
            let used = std::fs::read_to_string(&watched_file)
                .map(|records| records.contains("\"prompts_used\":true"))
                .unwrap_or(false);
            recorder
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push((!view.chat_prompts.is_empty(), used));
        },
    )));

    let mut timeline = Timeline::open(&phone, &room).await;
    phone
        .dispatch(Action::Timeline {
            action: TimelineAction::SendText {
                target: TimelineTarget {
                    room_id: room.clone(),
                    thread_root: None,
                },
                body: "你好".to_owned(),
                reply_to: None,
            },
        })
        .unwrap();
    timeline
        .until("消息已发出", |item| match &item.content {
            TimelineContent::Message { message } => match &message.body {
                MessageContent::Text { body, .. } if body == "你好" => Some(()),
                _ => None,
            },
            _ => None,
        })
        .await;
    phone
        .wait_onboarding_prompts("发过消息后提问收起", |view| {
            view.chat_prompts.is_empty().then_some(())
        })
        .await;

    let seen = seen.lock().unwrap();
    assert!(!seen.is_empty(), "房间记成发过消息后没有发布过");
    assert!(
        seen.iter().any(|(visible, _)| !visible),
        "提问没在这之后收起：{seen:?}"
    );
    assert!(
        seen.iter().all(|(visible, used)| *visible || *used),
        "视图收起候选提问的那次发布，prompts_used 还没落盘：{seen:?}"
    );
    let records = std::fs::read_to_string(&file).unwrap();
    assert!(
        records.contains(&room),
        "发过消息的房间要记进本机：{records}"
    );
    assert!(records.contains("\"prompts_used\":true"), "{records}");
}
