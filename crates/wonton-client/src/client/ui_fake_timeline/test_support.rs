//! 假时间线测试共用的房间、取批次的帮手和协议类型。
//!
//! 里面只有测试用的中文括号说明；下面这个标记也让界面文字检查（ADR-0043）把整个文件当测试代码，不进字符串目录。

#[cfg(test)]
const _: () = ();

pub(super) use super::*;

use crate::chat::TimelineSubscription;

pub(super) use crate::client::ui_fake_room::ITEMS;
pub(super) use wonton_protocol::{
    Action, MessageContent, MessageRun, MessageStatus, TimelineAction, TimelineContent,
    TimelineDiff, TimelineFocus, TimelineItem, TimelineScroll, TimelineTarget, TimelineUpdate,
    TimelineViewport,
};

pub(super) const ROOM: &str = "!ui-fake:home.example";

pub(super) fn target() -> TimelineTarget {
    TimelineTarget {
        room_id: ROOM.into(),
        thread_root: None,
    }
}

pub(super) async fn next(subscription: &TimelineSubscription) -> TimelineUpdate {
    subscription.next().await.expect("假房间一直开着")
}

pub(super) fn message(items: &[TimelineItem], id: &str) -> wonton_protocol::TimelineMessage {
    match &items.iter().find(|item| item.id == id).unwrap().content {
        TimelineContent::Message { message } => message.clone(),
        _ => panic!("{id} 不是消息"),
    }
}
