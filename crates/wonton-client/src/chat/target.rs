//! 按时间线（主对话或一个旁聊）分开记的东西（ADR-0093 §4、§5）：打开着的时间线、它的消息菜单和订阅。
//!
//! 同一目标的窗口共享 Matrix 时间线，每份订阅单独保存滚动位置与消息菜单。

use std::sync::{Arc, Mutex, Weak};

use wonton_l10n::l10n;
use wonton_matrix::{ChatRoom, OwnedEventId, OwnedRoomId};
use wonton_protocol::{CoreError, TimelineTarget};

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

    /// 错误信息里怎么写这条时间线。
    pub(crate) fn describe(&self) -> String {
        match &self.thread_root {
            Some(root) => format!("{} / {root}", self.room_id),
            None => self.room_id.to_string(),
        }
    }
}

/// 打开着的一条时间线。
pub(crate) struct OpenTimeline {
    pub(crate) key: TimelineKey,
    pub(crate) room: Weak<ChatRoom>,
    /// 这条时间线里每条消息的菜单，转发、撤回只认这里有的。
    pub(crate) menus: Weak<Mutex<Menus>>,
    /// 注册表不延长订阅生命周期；取消的订阅不再参与路由或菜单授权。
    pub(crate) subscription: Weak<TimelineSubscription>,
}

/// 所有仍存活的时间线订阅，不限制房间、旁聊或同一目标的订阅数。
#[derive(Default)]
pub(crate) struct OpenTimelines {
    open: Vec<OpenTimeline>,
}

impl OpenTimelines {
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
        for open in &self.open {
            if let Some(subscription) = open.subscription.upgrade() {
                subscription.cancel();
            }
        }
        self.open.clear();
    }
}

impl OpenTimeline {
    fn is_active(&self) -> bool {
        self.subscription
            .upgrade()
            .is_some_and(|subscription| subscription.is_active())
    }
}

/// 这条时间线没在订阅着。
pub(crate) fn not_subscribed(key: &TimelineKey) -> CoreError {
    CoreError::NotReady {
        reason: l10n!(
            CATALOG,
            language(),
            "chat-room-not-subscribed",
            room = key.describe()
        ),
    }
}
