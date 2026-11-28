//! 按时间线（主对话或一个旁聊）分开记的东西（ADR-0093 §4、§5）：打开着的时间线、它的消息菜单和订阅。
//!
//! 同一目标的窗口共享 Matrix 时间线，每份订阅独立保存滚动位置与消息菜单。

use std::borrow::Cow;
use std::sync::{Arc, Mutex, Weak};

use wonton_l10n::l10n;
use wonton_matrix::{ChatRoom, OwnedEventId, OwnedRoomId};
use wonton_protocol::{BotView, CoreError, RoomRow, TimelineTarget};

use super::menu::Menus;
use super::{TimelineSubscription, invalid};
use crate::l10n::{CATALOG, language};

/// 一条时间线：房间，加上旁聊的话题根（主对话是 `None`）。
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct TimelineKey {
    pub(crate) room_id: OwnedRoomId,
    pub(crate) thread_root: Option<OwnedEventId>,
}

impl TimelineKey {
    /// 读壳给的 [`TimelineTarget`]：房间 ID 和话题根都要合法。
    pub(crate) fn parse(target: &TimelineTarget) -> Result<Self, CoreError> {
        let room_id = target.room_id.parse::<OwnedRoomId>().map_err(|e| {
            invalid(&l10n!(
                CATALOG,
                language(),
                "chat-invalid-room-id",
                error = e.to_string()
            ))
        })?;
        let thread_root = target
            .thread_root
            .as_deref()
            .map(|root| {
                root.parse::<OwnedEventId>().map_err(|e| {
                    invalid(&l10n!(
                        CATALOG,
                        language(),
                        "chat-invalid-event-id",
                        error = e.to_string()
                    ))
                })
            })
            .transpose()?;
        Ok(Self {
            room_id,
            thread_root,
        })
    }

    pub(crate) fn main(room_id: OwnedRoomId) -> Self {
        Self {
            room_id,
            thread_root: None,
        }
    }

    pub(crate) fn thread(&self) -> Option<&wonton_matrix::EventId> {
        self.thread_root.as_deref()
    }
}

/// 打开着的一份时间线订阅。
pub(crate) struct OpenTimeline {
    pub(crate) id: u64,
    pub(crate) key: TimelineKey,
    pub(crate) room: Weak<ChatRoom>,
    pub(crate) menus: Weak<Mutex<Menus>>,
    pub(crate) subscription: Weak<TimelineSubscription>,
}

/// 订阅与在途打开按唯一编号区分；旧的取消不能移除新的订阅。
#[derive(Default)]
pub(crate) struct OpenTimelines {
    open: Vec<OpenTimeline>,
    next_id: u64,
    requests: Vec<(u64, TimelineKey)>,
}

impl OpenTimelines {
    pub(crate) fn begin(&mut self, key: &TimelineKey) -> u64 {
        self.next_id += 1;
        self.requests.push((self.next_id, key.clone()));
        self.next_id
    }

    pub(crate) fn contains_request(&self, id: u64) -> bool {
        self.requests.iter().any(|(open, _)| *open == id)
    }

    pub(crate) fn close(&mut self, id: u64) {
        self.requests.retain(|(open, _)| *open != id);
        self.open.retain(|open| open.id != id);
    }

    pub(crate) fn insert(&mut self, timeline: OpenTimeline) {
        self.open.retain(OpenTimeline::is_active);
        self.open.push(timeline);
    }

    pub(crate) fn get(&self, key: &TimelineKey) -> Option<&OpenTimeline> {
        self.open
            .iter()
            .find(|open| &open.key == key && open.is_active())
    }

    pub(crate) fn matching<'a>(
        &'a self,
        key: &'a TimelineKey,
    ) -> impl Iterator<Item = &'a OpenTimeline> {
        self.open
            .iter()
            .filter(move |open| &open.key == key && open.is_active())
    }

    /// 这个房间打开着的所有时间线，旁聊在前（刚点进去的那条）。
    pub(crate) fn all_in(&self, room_id: &OwnedRoomId) -> Vec<Arc<ChatRoom>> {
        let mut rooms: Vec<&OpenTimeline> = self
            .open
            .iter()
            .filter(|open| &open.key.room_id == room_id && open.is_active())
            .collect();
        rooms.sort_by_key(|open| open.key.thread_root.is_none());
        rooms
            .into_iter()
            .filter_map(|open| open.room.upgrade())
            .collect()
    }

    pub(crate) fn clear(&mut self) {
        self.open.clear();
        self.requests.clear();
    }
}

impl OpenTimeline {
    fn is_active(&self) -> bool {
        self.subscription
            .upgrade()
            .is_some_and(|subscription| subscription.is_active())
    }
}

/// 从发起打开到订阅取消共用同一个编号，旧订阅迟到的关闭不能关掉新订阅。
pub(crate) struct TimelineOpening {
    pub(crate) key: TimelineKey,
    pub(crate) id: u64,
    pub(crate) chat: Weak<super::ChatLink>,
}

impl Drop for TimelineOpening {
    fn drop(&mut self) {
        if let Some(chat) = self.chat.upgrade() {
            super::lock(&chat.open).close(self.id);
        }
    }
}

/// 这条时间线没在订阅着。`conversation` 是给人看的会话叫法，见 [`conversation_label`]。
pub(crate) fn not_subscribed(conversation: &str) -> CoreError {
    CoreError::NotReady {
        reason: l10n!(
            CATALOG,
            language(),
            "chat-room-not-subscribed",
            conversation = conversation
        ),
    }
}

/// 弹给用户看的会话叫法：房间名。名字取不到时用对方的名字、会话类型，都没有时是“这个会话”。
///
/// 房间 ID、用户 ID 和 SDK 的 `Empty Room` 占位名都不算名字。
pub(crate) fn conversation_label<'a>(
    rooms: &'a [RoomRow],
    bots: &'a [BotView],
    key: &TimelineKey,
) -> Cow<'a, str> {
    let Some(row) = rooms.iter().find(|row| row.room_id == key.room_id.as_str()) else {
        return Cow::Owned(l10n!(CATALOG, language(), "chat-conversation-this"));
    };
    if let Some(name) = usable_name(&row.name, &row.room_id) {
        return Cow::Borrowed(name);
    }
    if let Some(name) = row
        .direct_user_id
        .as_deref()
        .and_then(|user| bots.iter().find(|bot| bot.user_id == user))
        .and_then(|bot| usable_name(&bot.name, &row.room_id))
    {
        return Cow::Borrowed(name);
    }
    Cow::Owned(l10n!(
        CATALOG,
        language(),
        if row.is_direct {
            "chat-conversation-direct"
        } else {
            "chat-conversation-group"
        }
    ))
}

/// 这个名字能不能拿来指这个会话：空白、原始 ID、SDK 的占位名都不能。
fn usable_name<'a>(name: &'a str, room_id: &str) -> Option<&'a str> {
    let name = name.trim();
    let placeholder = name.is_empty()
        || name == room_id
        || name.starts_with('@')
        || name.starts_with("Empty Room");
    (!placeholder).then_some(name)
}

#[cfg(test)]
#[path = "target_tests.rs"]
mod target_tests;
