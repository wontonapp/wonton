//! 引导的最后一步依赖真实聊天：主人在和默认 Agent 的私聊里发过第一条消息后，候选提问就不再给，
//! 发过消息的房间记进本机（ADR-0034）。

use serde_json::json;
use wonton_client::{Action, MessageContent, TimelineContent};

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
    let room_id = accept_the_bots_invite(&phone, &bot).await;

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
        .dispatch(Action::SendText {
            room_id: room.clone(),
            body: "你好".to_owned(),
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
    let records = std::fs::read_to_string(dir.path().join("onboarding.json")).unwrap();
    assert!(
        records.contains(&room),
        "发过消息的房间要记进本机：{records}"
    );
    assert!(records.contains("\"prompts_used\":true"));
}
