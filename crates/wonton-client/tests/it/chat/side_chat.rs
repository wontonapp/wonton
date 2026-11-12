//! 旁聊（ADR-0093）：Bot 声明支持后主人能开旁聊；主对话里旁聊是一张卡片，话题里的回复不出现，同一个旁聊只显示
//! 最新一条结果行；旁聊的时间线有自己的头部、草稿和未读，房间的未读是主对话加各旁聊。根发不出去时报错，不留空旁聊。

use serde_json::json;
use wonton_client::{
    Action, CoreError, MessageContent, TimelineContent, TimelineItem, TimelineScroll,
};
use wonton_matrix::{ChatRoom, MatrixSession, MessageBody, OwnedRoomId};
use wonton_protocol::{
    BotActivityFacts, SideChatCard, SideChatState, TimelineAction, TimelineTarget,
};

use super::bot_receives;
use super::confirmation::joined_direct_chat;
use super::support::{Home, Phone, Timeline};
use crate::plugins::rig::running;

const PLUGIN: &str = "app.bot";

struct Setup {
    _dir: tempfile::TempDir,
    phone: Phone,
    home: Home,
    bot: MatrixSession,
    room: OwnedRoomId,
    bot_room: ChatRoom,
}

/// 装了一个 Bot 插件，主人和 Bot 有私聊，Bot 在房间里声明了支持旁聊，聊天列表那一行认出来了。
async fn setup() -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    let bot = home.bot("hermes", "Hermes").await;
    let mut installed = running(PLUGIN);
    installed.bot_user_id = Some(bot.user_id().to_string());
    home.set_installed_plugins(vec![installed]);
    home.bot_api.facts(BotActivityFacts {
        plugin_id: PLUGIN.to_owned(),
        bot: Some("default".into()),
        user_id: bot.user_id().to_string(),
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
                .any(|known| known.user_id == *bot.user_id())
                .then_some(())
        })
        .await;
    let room = joined_direct_chat(&phone, &bot).await;
    let bot_room = bot.open_room(&room).await.expect("Bot 打开房间");
    bot_room
        .send_state_event_raw(
            "app.wonton.bot.side_chats",
            bot.user_id().as_str(),
            json!({"v": 1}),
        )
        .await
        .expect("Bot 声明支持旁聊");
    Setup {
        _dir: dir,
        phone,
        home,
        bot,
        room,
        bot_room,
    }
}

/// 打开主对话（打开时核心重读支持声明），等聊天列表那一行认出支持旁聊、记下是和 Bot 的私聊。
async fn open_main(setup: &Setup) -> Timeline {
    let timeline = Timeline::open(&setup.phone, setup.room.as_str()).await;
    setup
        .phone
        .wait_chat("这一行支持旁聊", |view| {
            view.rooms
                .iter()
                .any(|row| {
                    row.room_id == setup.room.as_str()
                        && row.side_chats
                        && row.direct_user_id.as_deref() == Some(setup.bot.user_id().as_str())
                })
                .then_some(())
        })
        .await;
    timeline
}

fn card(item: &TimelineItem, root: &str) -> Option<SideChatCard> {
    match &item.content {
        TimelineContent::SideChat { card } if card.root_event_id == root => Some(card.clone()),
        _ => None,
    }
}

fn has_text(items: &[TimelineItem], text: &str) -> bool {
    items.iter().any(|item| {
        matches!(&item.content, TimelineContent::Message { message }
            if matches!(&message.body, MessageContent::Text { body, .. } if body == text))
    })
}

fn result_states(items: &[TimelineItem], root: &str) -> Vec<SideChatState> {
    items
        .iter()
        .filter_map(|item| match &item.content {
            TimelineContent::SideChatResult {
                root_event_id,
                state,
                ..
            } if root_event_id == root => Some(*state),
            _ => None,
        })
        .collect()
}

fn result_line(root: &str, state: &str) -> serde_json::Value {
    json!({
        "msgtype": "m.notice",
        "body": format!("整理照片：{state}"),
        "app.wonton.side_chat_result": {"v": 1, "root": root, "title": "整理照片", "state": state},
    })
}

async fn room_unread(phone: &Phone, room: &OwnedRoomId, want: u64) {
    phone
        .wait_chat(&format!("这一行的未读是 {want}"), |view| {
            view.rooms
                .iter()
                .any(|row| row.room_id == room.as_str() && row.unread_count == want)
                .then_some(())
        })
        .await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_side_chat_is_a_card_in_the_main_timeline_with_its_own_timeline() {
    let setup = setup().await;
    let mut main = open_main(&setup).await;
    let contacts = setup.phone.chat_contacts(String::new());
    assert!(
        contacts
            .iter()
            .any(|contact| contact.user_id == setup.bot.user_id().as_str()
                && contact.side_chat_room_id.as_deref() == Some(setup.room.as_str())),
        "联系人里的 Bot 带上能开旁聊的房间：{contacts:?}"
    );

    // 开旁聊：根是卡片，第一句话进了话题。
    let root = setup
        .phone
        .start_side_chat(
            setup.room.to_string(),
            "整理照片\n把上周的都放进相册".to_owned(),
        )
        .await
        .expect("开旁聊");
    let first = main
        .until("主对话里有旁聊卡", |item| card(item, &root))
        .await;
    assert_eq!(first.title, "整理照片");
    let root_id = root.parse().unwrap();
    bot_receives(
        &setup.bot_room,
        "Bot 在话题里收到第一句",
        |message| {
            (message.body == MessageBody::Text("整理照片\n把上周的都放进相册".into())
                && message.thread_root.as_ref() == Some(&root_id))
            .then_some(())
        },
    )
    .await;

    // Bot 在话题里回：主对话里不出现，卡片的回复数和未读跟着变；房间的未读算上它。
    setup
        .bot_room
        .send_text_in_thread(&root_id, "在整理了")
        .await
        .expect("Bot 在话题里回");
    main.until("卡片上有 Bot 的回复", |item| {
        card(item, &root).filter(|card| card.reply_count == 2 && card.unread_count == 1)
    })
    .await;
    assert!(
        !has_text(&main.items, "在整理了"),
        "话题里的回复不在主对话里"
    );
    room_unread(&setup.phone, &setup.room, 1).await;

    // 两条结果行：主对话只显示最新的一条，卡片按它。
    for state in ["failed", "done"] {
        setup
            .bot_room
            .send_raw_message(result_line(&root, state))
            .await
            .expect("Bot 发结果行");
    }
    main.until_items("只剩最新的结果行", |items| {
        (result_states(items, &root) == [SideChatState::Done]).then_some(())
    })
    .await;
    main.until("卡片是已完成", |item| {
        card(item, &root).filter(|card| card.state == SideChatState::Done)
    })
    .await;

    // 旁聊的时间线：头部是标题和状态，根不是条目，话题里的消息在。
    let mut thread = Timeline::open_target(&setup.phone, setup.room.as_str(), Some(&root)).await;
    thread
        .until_items("旁聊里有两句", |items| {
            (has_text(items, "在整理了") && has_text(items, "整理照片\n把上周的都放进相册"))
                .then_some(())
        })
        .await;
    let header = thread.side_chat.clone().expect("旁聊时间线有头部");
    assert_eq!(header.title, "整理照片");
    assert!(thread.items.iter().all(|item| card(item, &root).is_none()));

    side_chat_drafts_are_separate(&setup.phone, setup.room.as_str(), &root).await;

    // 主对话标已读（结果行读过了）：房间的未读只剩旁聊那一条；再在旁聊里标已读，房间的未读清零。
    mark_read(&setup.phone, &mut main, setup.room.as_str(), None).await;
    room_unread(&setup.phone, &setup.room, 1).await;
    mark_read(&setup.phone, &mut thread, setup.room.as_str(), Some(&root)).await;
    room_unread(&setup.phone, &setup.room, 0).await;
}

/// 像壳一样：列表在底部时（先报告到底）发已读。
async fn mark_read(phone: &Phone, timeline: &mut Timeline, room: &str, thread_root: Option<&str>) {
    timeline.report_scroll(TimelineScroll::Bottom).await;
    phone
        .dispatch(Action::Timeline {
            action: TimelineAction::MarkRead {
                target: TimelineTarget {
                    room_id: room.to_owned(),
                    thread_root: thread_root.map(str::to_owned),
                },
            },
        })
        .unwrap();
}

/// 主对话和旁聊各有一份草稿，互不覆盖。
async fn side_chat_drafts_are_separate(phone: &Phone, room: &str, root: &str) {
    let target = |thread_root: Option<&str>| TimelineTarget {
        room_id: room.to_owned(),
        thread_root: thread_root.map(str::to_owned),
    };
    for (thread_root, body) in [(None, "主对话的草稿"), (Some(root), "旁聊的草稿")] {
        phone
            .dispatch(Action::Timeline {
                action: TimelineAction::SetDraft {
                    target: target(thread_root),
                    body: body.to_owned(),
                    reply_to: None,
                },
            })
            .unwrap();
    }
    let waiting = async {
        loop {
            let main = phone.draft(target(None)).await.ok().flatten();
            let side = phone.draft(target(Some(root))).await.ok().flatten();
            if main.is_some_and(|draft| draft.body == "主对话的草稿")
                && side.is_some_and(|draft| draft.body == "旁聊的草稿")
            {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    };
    tokio::time::timeout(super::support::WAIT, waiting)
        .await
        .expect("主对话和旁聊各有自己的草稿");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_root_that_fails_to_send_leaves_no_side_chat() {
    let setup = setup().await;
    let mut main = open_main(&setup).await;

    let empty = setup
        .phone
        .start_side_chat(setup.room.to_string(), "  \n ".to_owned())
        .await;
    assert!(
        matches!(empty, Err(CoreError::InvalidInput { .. })),
        "{empty:?}"
    );

    setup.home.fail_sends(true);
    let failed = setup
        .phone
        .start_side_chat(setup.room.to_string(), "整理照片".to_owned())
        .await;
    assert!(
        matches!(failed, Err(ref error) if !matches!(error, CoreError::InvalidInput { .. })),
        "根发不出去按原因报错：{failed:?}"
    );
    setup.home.fail_sends(false);

    // 之后照常聊天；房间里没有旁聊卡，第一句话也没发出去。
    setup.bot_room.send_text("还在").await.expect("Bot 发文字");
    main.until_items("收到 Bot 的消息", |items| {
        has_text(items, "还在").then_some(())
    })
    .await;
    assert!(
        main.items
            .iter()
            .all(|item| !matches!(item.content, TimelineContent::SideChat { .. })),
        "{:#?}",
        main.items
    );
    let messages = setup.bot_room.messages().await;
    assert!(
        messages.iter().all(
            |message| message.body != MessageBody::Text("整理照片".into())
                && message.body != MessageBody::Notice("整理照片".into())
        ),
        "根和第一句话都没发出去：{messages:#?}"
    );
}
