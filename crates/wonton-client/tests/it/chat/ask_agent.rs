//! Siri、快捷指令的“问 Bot”：一句话直接发给默认 Agent。

use serde_json::json;
use wonton_client::{Action, CoreError, MessageContent, RoomMembership, TimelineContent};

use wonton_matrix::OwnedUserId;

use super::support::{Home, OWNER, Phone, Timeline};

/// Bot 发起了私聊、主人还没接受：问一句就接受邀请并发出去，壳打开的就是这个私聊。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn asking_joins_the_bots_chat_and_sends_the_question() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    // 还没配对、没有默认 Agent 时如实拒绝，不悄悄丢掉这句话。
    assert!(matches!(
        phone.dispatch(Action::Chat {
            action: wonton_protocol::ChatAction::AskAgent {
                text: "明天下雨吗".to_owned()
            }
        }),
        Err(CoreError::NotReady { .. })
    ));

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
        "prompts": [],
        "catalog_problem": null
    }));
    phone.connect(&home);
    phone.signed_in().await;

    let bot = home.bot("hermes", "Hermes").await;
    let owner: OwnedUserId = OWNER.parse().unwrap();
    let room_id = bot
        .create_direct_chat(&owner, true)
        .await
        .expect("Bot 发起加密私聊");
    let room = room_id.to_string();
    phone
        .wait_chat("收到 Bot 的邀请", |view| {
            view.rooms
                .iter()
                .any(|row| {
                    row.room_id == room
                        && row.membership == RoomMembership::Invited
                        && row.direct_user_id.as_deref() == Some(bot.user_id().as_str())
                })
                .then_some(())
        })
        .await;
    let primary = phone
        .wait_onboarding_prompts("默认 Agent 就绪", |view| {
            view.primary_bot_user_id.clone()
        })
        .await;
    assert_eq!(primary, bot.user_id().as_str());

    phone
        .dispatch(Action::Chat {
            action: wonton_protocol::ChatAction::AskAgent {
                text: "  明天下雨吗 ".to_owned(),
            },
        })
        .unwrap();
    phone
        .wait_chat("接受了 Bot 的邀请", |view| {
            view.rooms
                .iter()
                .any(|row| row.room_id == room && row.membership == RoomMembership::Joined)
                .then_some(())
        })
        .await;
    let mut timeline = Timeline::open(&phone, &room).await;
    timeline
        .until("问题发进了 Bot 的私聊", |item| {
            match &item.content {
                TimelineContent::Message { message } => match &message.body {
                    MessageContent::Text { body, .. } if body == "明天下雨吗" => Some(()),
                    _ => None,
                },
                _ => None,
            }
        })
        .await;
}
