//! 正在输入、命令表和跳到原消息（ADR-0109 §4、§8）。

use std::time::Duration;

use serde_json::json;
use wonton_client::{Action, MessageContent, TimelineContent, TimelineScroll, TimelineUpdate};
use wonton_matrix::{MatrixSession, OwnedRoomId, UserId};
use wonton_protocol::{BOT_COMMANDS_EVENT, JumpState, RoomCommand, TimelineAction, TimelineTarget};

use super::bot_profile_support::typing;
use super::message_actions::join_group_room;
use super::support::{Home, Phone, Timeline, WAIT, is_room_list};

/// 等一批满足条件的更新，交回去。
async fn until_update(
    timeline: &mut Timeline,
    what: &str,
    mut check: impl FnMut(&TimelineUpdate) -> bool,
) -> TimelineUpdate {
    let waiting = async {
        loop {
            let update = timeline.next().await;
            if check(&update) {
                return update;
            }
        }
    };
    tokio::time::timeout(WAIT, waiting)
        .await
        .unwrap_or_else(|_| panic!("{what}：{WAIT:?} 内没等到"))
}

fn target(room_id: &str) -> TimelineTarget {
    TimelineTarget {
        room_id: room_id.to_owned(),
        thread_root: None,
    }
}

/// Bot 写一张命令表：`key` 是 state_key，可以是自己的或别人的。
async fn write_table(bot: &MatrixSession, room_id: &OwnedRoomId, key: &UserId, names: &[&str]) {
    let room = bot.open_room(room_id).await.expect("Bot 打开群聊");
    let commands: Vec<_> = names
        .iter()
        .map(|name| json!({"name": name, "description": "做点什么"}))
        .collect();
    room.send_state_event_raw(
        BOT_COMMANDS_EVENT,
        key.as_str(),
        json!({"commands": commands}),
    )
    .await
    .expect("Bot 写命令表");
}

fn by_name<'a>(commands: &'a [RoomCommand], name: &str) -> &'a RoomCommand {
    commands
        .iter()
        .find(|command| command.name == name)
        .unwrap_or_else(|| panic!("没有 {name}，现在是 {commands:?}"))
}

/// 行为：Bot 正在输入时名单里有它，它一发消息就马上清掉，不留到 30 秒超时。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn typing_shows_the_bot_and_clears_when_it_sends() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    phone.signed_in().await;
    let bot = home.bot("hermes", "Hermes").await;
    let room_id = join_group_room(&phone, &bot, "Group").await;
    let room = room_id.to_string();
    let mut timeline = Timeline::open(&phone, &room).await;
    let bot_room = bot.open_room(&room_id).await.expect("Bot 打开群聊");

    // 房间的订阅这一刻才在同步请求里挂上：在那之前发的正在输入没有谁订阅，服务器不会再补。
    let subscribed = async {
        loop {
            if home.sync_requests().iter().any(|request| {
                is_room_list(&request["conn_id"])
                    && request["room_subscriptions"][room.as_str()].is_object()
            }) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    };
    tokio::time::timeout(WAIT, subscribed)
        .await
        .expect("时间线订阅没有出现在同步请求里");

    // 正在输入是一次性的 ephemeral 事件，重报直到看见；已经标着“正在输入”时服务器不重发，撤一下再报才算一次变化。
    let update = 'seen: {
        for attempt in 1..=12 {
            if attempt > 1 {
                typing(&home.matrix.url, &bot, &room, false).await;
            }
            typing(&home.matrix.url, &bot, &room, true).await;
            let seen = tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    let update = timeline.next().await;
                    if update.typing.iter().any(|name| name == "Hermes") {
                        return update;
                    }
                }
            })
            .await;
            if let Ok(update) = seen {
                break 'seen update;
            }
            assert!(attempt < 12, "重报 {attempt} 次正在输入都没收到");
        }
        unreachable!("上一轮没收到的断言会先报出来")
    };
    assert_eq!(update.typing, ["Hermes"]);

    bot_room.send_text("我来了", &[]).await.expect("Bot 发文字");
    until_update(&mut timeline, "发消息后清掉正在输入", |update| {
        update.typing.is_empty()
    })
    .await;
}

/// 行为：两个 Bot 的表都列出来、各自带名字；不合法的条目记进 `commands_omitted`。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn command_tables_of_both_bots_are_merged() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    phone.signed_in().await;
    let first = home.bot("alpha", "Alpha").await;
    let second = home.bot("beta", "Beta").await;
    let room_id = join_group_room(&phone, &first, "Group").await;
    let room = room_id.to_string();
    let first_room = first.open_room(&room_id).await.expect("Bot 打开群聊");
    first_room
        .invite(second.user_id())
        .await
        .expect("邀请第二个 Bot");
    second.join_room(&room_id).await.expect("第二个 Bot 加入");
    // 新成员默认 power level 0，写自定义状态事件要 50：把默认门槛放开，两个 Bot 都能写自己的表。
    let mut levels = first_room
        .fetch_state_event_content("m.room.power_levels", "")
        .await
        .expect("取权限")
        .expect("房间有权限事件");
    levels["state_default"] = 0.into();
    first_room
        .send_state_event_raw("m.room.power_levels", "", levels)
        .await
        .expect("放开状态事件权限");
    let mut timeline = Timeline::open(&phone, &room).await;

    write_table(&first, &room_id, first.user_id(), &["go", "/slash"]).await;
    write_table(&second, &room_id, second.user_id(), &["help"]).await;

    let update = until_update(&mut timeline, "收到两张命令表", |update| {
        update.commands.len() == 2
    })
    .await;
    assert_eq!(by_name(&update.commands, "go").bot_name, "Alpha");
    assert_eq!(by_name(&update.commands, "help").bot_name, "Beta");
    assert_eq!(by_name(&update.commands, "go").text, "/go");
    assert_eq!(update.commands_omitted, 1);
}

/// 行为：60 条消息之后跳到第 1 条，没加载就一直往回翻，找到后给带高亮的焦点、`back_to` 是跳之前那条；
/// 再回来焦点落回原处、栈空；滚到底清栈。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn jumping_to_an_unloaded_message_pages_back_and_comes_back() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    phone.signed_in().await;
    let bot = home.bot("hermes", "Hermes").await;
    let room_id = join_group_room(&phone, &bot, "Group").await;
    let room = room_id.to_string();
    let bot_room = bot.open_room(&room_id).await.expect("Bot 打开群聊");
    let mut first = String::new();
    for index in 0..60 {
        let event_id = bot_room
            .send_text_event(&format!("第 {index} 条"))
            .await
            .expect("Bot 发文字");
        if index == 0 {
            first = event_id.to_string();
        }
    }
    let mut timeline = Timeline::open(&phone, &room).await;
    let last_item = timeline
        .until("等到最后一条", |item| match &item.content {
            TimelineContent::Message { message }
                if matches!(&message.body, MessageContent::Text { body, .. } if body == "第 59 条") =>
            {
                Some(item.id.clone())
            }
            _ => None,
        })
        .await;
    assert!(
        !timeline.items.iter().any(|item| matches!(&item.content, TimelineContent::Message { message } if message.event_id.as_deref() == Some(first.as_str()))),
        "第 1 条不该已经在列表里"
    );

    phone
        .dispatch(Action::Timeline {
            action: TimelineAction::JumpToMessage {
                target: target(&room),
                event_id: first.clone(),
                from_item_id: last_item.clone(),
            },
        })
        .unwrap();
    // 第 1 条不在已加载的条目里：先 Loading，翻到它才给焦点。
    let mut saw_loading = false;
    let jumped = {
        let waiting = async {
            loop {
                let update = timeline.next().await;
                saw_loading |= update.jump == JumpState::Loading;
                if update.jump == JumpState::Idle && update.focus.is_some() {
                    return update;
                }
                if update.jump == JumpState::Unavailable {
                    return update;
                }
            }
        };
        tokio::time::timeout(WAIT, waiting)
            .await
            .expect("跳不到第 1 条")
    };
    assert!(saw_loading, "没先报 Loading");
    let focus = jumped.focus.expect("跳到了要给焦点");
    assert!(focus.highlight);
    assert_eq!(jumped.back_to.as_deref(), Some(last_item.as_str()));
    let found = timeline
        .items
        .iter()
        .find(|item| item.id == focus.item_id)
        .expect("焦点那条在列表里");
    assert!(
        matches!(&found.content, TimelineContent::Message { message } if message.event_id.as_deref() == Some(first.as_str()))
    );

    // 滚到底清栈，再“回来”就没有目标，直接滚到底。
    timeline.report_scroll(TimelineScroll::Bottom).await;
    until_update(&mut timeline, "滚到底清掉栈", |update| {
        update.back_to.is_none() && update.jump == JumpState::Idle
    })
    .await;
    phone
        .dispatch(Action::Timeline {
            action: TimelineAction::JumpBack {
                target: target(&room),
            },
        })
        .unwrap();
    until_update(&mut timeline, "没得回来就滚到底", |update| {
        update.scroll_to_bottom
    })
    .await;
}

/// 行为：跳一条永远不出现的，翻到开头就是 Unavailable。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn jumping_to_a_missing_message_ends_unavailable() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    phone.signed_in().await;
    let bot = home.bot("hermes", "Hermes").await;
    let room_id = join_group_room(&phone, &bot, "Group").await;
    let room = room_id.to_string();
    let bot_room = bot.open_room(&room_id).await.expect("Bot 打开群聊");
    bot_room
        .send_text("只有这一条", &[])
        .await
        .expect("Bot 发文字");
    let mut timeline = Timeline::open(&phone, &room).await;
    let item = timeline
        .until("等到那一条", |item| match &item.content {
            TimelineContent::Message { message } if message.event_id.is_some() => {
                Some(item.id.clone())
            }
            _ => None,
        })
        .await;

    phone
        .dispatch(Action::Timeline {
            action: TimelineAction::JumpToMessage {
                target: target(&room),
                event_id: "$nonexistent:home.example".to_owned(),
                from_item_id: item,
            },
        })
        .unwrap();
    let update = until_update(&mut timeline, "翻到开头", |update| {
        update.jump == JumpState::Unavailable
    })
    .await;
    assert_eq!(update.focus, None);
}
