//! 整个房间和旁聊的批次：第一批的全部条目、旁聊卡片和刚开旁聊的一批。

use wonton_protocol::{
    BackPagination, ConfirmationView, RoomCommand, SideChatCard, SideChatState, TimelineContent,
    TimelineDiff, TimelineItem, TimelineUpdate, TimelineViewport,
};

use super::super::{ui_fake, ui_fake_devices};
use super::item::item;
use super::layout::{
    DAY_BREAK_INDEX, ECHO, FIRST_AT_MS, ITEMS, OWNER, SIDE_ITEMS, SIDE_ROOT_INDEX, TOP_INDEX, at_ms,
};
use super::message::text_message;
use super::pages::panel;

/// 房间的第一批：全部条目，落点在第三条。之后的批次由 `ui_fake_timeline` 按壳的操作推。
/// 通话中的那个群聊（`ui_fake::CALL_ROOM_ID`）另带顶栏的 widget 入口和“通话中”。
/// Echo 一直在输入，带三条命令（ADR-0109 的命令表和正在输入）。
/// 跨日的地方像核心一样先放一条日期分隔：第一条前和 `DAY_BREAK_INDEX` 前各一条；第一条前那条和它上面的
/// `TimelineStart` 排在一起，和真核心翻到房间开头时的样子一样。
pub(crate) fn timeline(room_id: &str) -> TimelineUpdate {
    let in_call = room_id == ui_fake::CALL_ROOM_ID;
    let mut items: Vec<TimelineItem> = Vec::with_capacity(ITEMS + 3);
    items.push(timeline_start());
    items.push(divider(TOP_INDEX));
    for index in 0..ITEMS {
        if index == DAY_BREAK_INDEX {
            items.push(divider(index));
        }
        items.push(item(index));
    }
    // 走查进房落点用：`WONTON_UI_FAKE_ANCHOR=last` 让落点就是最新一条，`placeAnchor` 走贴底那一支。
    let anchor = if std::env::var("WONTON_UI_FAKE_ANCHOR").as_deref() == Ok("last") {
        items.last().map(|item| item.id.clone())
    } else {
        Some("ui-fake-2".into())
    };
    TimelineUpdate {
        unread_jump: None,
        anchor,
        diffs: vec![TimelineDiff::Reset { items }],
        back_pagination: BackPagination::ReachedStart,
        viewport: TimelineViewport::Away { new_messages: 0 },
        scroll_to_bottom: false,
        panel: panel(),
        panel_selected: None,
        key_notice: None,
        // Echo 的房间里挂着一张等回答的确认卡片，输入框上方的入口由界面测试看。
        confirmations: if room_id == ui_fake::ROOM_ID {
            vec![ConfirmationView {
                card: ui_fake_devices::card(),
                sending: None,
                sending_choice_id: None,
                problem: None,
                window: None,
            }]
        } else {
            Vec::new()
        },
        side_chat: None,
        widgets: if in_call {
            vec![ui_fake::call_widget()]
        } else {
            Vec::new()
        },
        call_active: in_call,
        // UI 测试的假房间：私聊（顶栏显示 Bot）不带；通话群是群聊，带一个在场的 AI。
        // 假数据的成员名单视为已取全，所以都是 Some。
        room_bots: if in_call {
            Some(vec!["@hermes:home.example".into()])
        } else {
            Some(Vec::new())
        },
        typing: vec!["Echo".into()],
        commands: commands(),
        commands_omitted: 0,
        focus: None,
        jump: wonton_protocol::JumpState::Idle,
        back_to: None,
        editing: None,
        focus_revision: 0,
    }
}

/// 挂在 [`SIDE_ROOT_INDEX`] 那条消息上的旁聊卡片。
pub(crate) fn side_card() -> SideChatCard {
    SideChatCard {
        root_event_id: format!("$ui-fake-{SIDE_ROOT_INDEX}"),
        title: "Fixture side chat".into(),
        reply_count: SIDE_ITEMS as u64,
        unread_count: 0,
        state: SideChatState::Open,
    }
}

/// 旁聊里的一批：几条短回复，最底下一条是落点，顶上是这个旁聊的标题和状态。
pub(crate) fn side_timeline(room_id: &str) -> TimelineUpdate {
    let items: Vec<TimelineItem> = (0..SIDE_ITEMS)
        .map(|index| {
            let own = index % 2 == 1;
            let message = text_message(
                &format!("side-{index}"),
                if own { OWNER } else { ECHO },
                FIRST_AT_MS + index as u64 * 60_000,
                &[format!("Fixture side {index}")],
            );
            TimelineItem {
                id: format!("side-{index}"),
                content: TimelineContent::Message { message },
            }
        })
        .collect();
    let mut update = timeline(room_id);
    update.anchor = items.last().map(|item| item.id.clone());
    update.viewport = TimelineViewport::Bottom;
    update.diffs = vec![TimelineDiff::Reset { items }];
    update.side_chat = Some(side_card());
    update.typing = Vec::new();
    update
}

/// 刚开的旁聊的一批：只有主人发出去的第一句。假入口没有服务器，根和这条都由 `start_side_chat` 现造。
pub(crate) fn started_timeline(room_id: &str, body: &str) -> TimelineUpdate {
    let message = text_message("side-new-0", OWNER, FIRST_AT_MS, &[body.to_owned()]);
    let items = vec![TimelineItem {
        id: "side-new-0".into(),
        content: TimelineContent::Message { message },
    }];
    let mut update = timeline(room_id);
    update.anchor = Some("side-new-0".into());
    update.viewport = TimelineViewport::Bottom;
    update.diffs = vec![TimelineDiff::Reset { items }];
    update.side_chat = Some(side_card());
    update.typing = Vec::new();
    update
}

fn commands() -> Vec<RoomCommand> {
    [
        ("help", "Show help", None),
        ("model", "Switch model", Some("model name")),
        ("new", "Start over", None),
        ("photo album", "Pick from an album", None),
        ("summarize-everything-in-this-thread", "Long summary", None),
    ]
    .into_iter()
    .map(|(name, description, hint)| RoomCommand {
        bot_user_id: ECHO.into(),
        bot_name: "Echo".into(),
        name: name.into(),
        description: description.into(),
        hint: hint.map(Into::into),
        text: format!("/{name}"),
    })
    .collect()
}

/// 日期分隔：核心在跨日处发的虚拟条目，时间取那一天的第一条消息（`index` 就是它）。
fn divider(index: usize) -> TimelineItem {
    TimelineItem {
        id: format!("ui-fake-day-{index}"),
        content: TimelineContent::DateDivider {
            timestamp_ms: at_ms(index),
        },
    }
}

/// 房间开头：SDK 固定放在最前面的条目，翻到头时它上面是那条分隔。
fn timeline_start() -> TimelineItem {
    TimelineItem {
        id: "ui-fake-start".into(),
        content: TimelineContent::TimelineStart,
    }
}
