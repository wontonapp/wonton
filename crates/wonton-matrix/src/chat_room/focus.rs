//! 时间线看房间的哪一段（ADR-0093 §4）：平铺整个房间（桥、后台跟着的房间）、主对话（话题回复收起）、一个话题。

use matrix_sdk::{
    Room,
    ruma::{EventId, OwnedEventId},
};
use matrix_sdk_ui::timeline::{RoomExt, Timeline, TimelineFocus};
use serde_json::Value;

use super::ChatRoom;
use crate::{MatrixSession, Result, timeline};

/// 时间线看哪一段。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Focus {
    /// 整个房间平铺，话题回复也在里面：桥按话题根分会话要读到它们。
    Flat,
    /// 主对话：话题回复不显示，话题根带回复数；已读回执挂在 `main` 上。
    Main,
    /// 一个话题，值是话题根：发出去的文字和图片自动带 `m.thread` 关系，已读回执挂在话题根上。
    Thread(OwnedEventId),
}

pub(crate) async fn build_timeline(room: &Room, focus: &Focus) -> Result<Timeline> {
    let builder = room.timeline_builder().event_filter(timeline::shown);
    let builder = match focus {
        Focus::Flat => builder,
        Focus::Main => builder.with_focus(TimelineFocus::Live {
            hide_threaded_events: true,
        }),
        Focus::Thread(root) => builder.with_focus(TimelineFocus::Thread {
            root_event_id: root.clone(),
        }),
    };
    Ok(builder.build().await?)
}

/// 原始事件是不是话题回复（`m.relates_to.rel_type` 是 `m.thread`）。
pub(crate) fn in_thread(json: &str) -> bool {
    serde_json::from_str::<Value>(json).is_ok_and(|event| {
        event["content"]["m.relates_to"]["rel_type"].as_str() == Some("m.thread")
    })
}

impl ChatRoom {
    /// 这条时间线是哪个话题的；主对话和平铺的是 `None`。
    pub fn thread_root(&self) -> Option<&EventId> {
        match &self.focus {
            Focus::Thread(root) => Some(root),
            Focus::Flat | Focus::Main => None,
        }
    }
}

impl MatrixSession {
    /// 打开一个房间的主对话：和 [`Self::open_room`] 一样订阅这个房间，时间线里话题回复收起，话题根带回复数
    /// （ADR-0093 §4）。App 的聊天页用它；桥要读话题回复，用平铺的 [`Self::open_room`]、[`Self::watch_room`]。
    pub async fn open_main(&self, room_id: &matrix_sdk::ruma::RoomId) -> Result<ChatRoom> {
        let room = self.subscribe_opened(room_id).await?;
        ChatRoom::open_with(room, Focus::Main).await
    }

    /// 打开一个房间里的一个话题（旁聊）。房间订阅同 [`Self::open_main`]，主对话和话题可以同时开着。
    pub async fn open_thread(
        &self,
        room_id: &matrix_sdk::ruma::RoomId,
        root: &EventId,
    ) -> Result<ChatRoom> {
        let room = self.subscribe_opened(room_id).await?;
        ChatRoom::open_with(room, Focus::Thread(root.to_owned())).await
    }
}
