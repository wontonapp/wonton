//! 按时间线（主对话或一个旁聊）分开记的东西（ADR-0093 §4、§5）：打开着的时间线、它的消息菜单和订阅。
//!
//! 同一时间只开一个房间；同一个房间里主对话和一个旁聊可以同时开着（从主对话点进旁聊，返回时主对话还在）。

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
    pub(crate) room: Arc<ChatRoom>,
    /// 这条时间线里每条消息的菜单，转发、撤回只认这里有的。
    pub(crate) menus: Arc<Mutex<Menus>>,
    /// 订阅，用来把壳报告的滚动位置送进去。订阅没了（壳重开、页面关掉）就升级不出来。
    pub(crate) subscription: Weak<TimelineSubscription>,
}

/// 打开着的时间线：同一个房间的主对话和至多一个旁聊。
#[derive(Default)]
pub(crate) struct OpenTimelines {
    open: Vec<OpenTimeline>,
    next_id: u64,
    requests: Vec<(u64, TimelineKey)>,
}

impl OpenTimelines {
    pub(crate) fn begin(&mut self, key: &TimelineKey) -> u64 {
        self.requests.retain(|(_, open)| {
            open.room_id == key.room_id
                && open != key
                && (open.thread_root.is_none() || key.thread_root.is_none())
        });
        self.open
            .retain(|open| self.requests.iter().any(|(_, key)| key == &open.key));
        self.next_id += 1;
        self.requests.push((self.next_id, key.clone()));
        self.next_id
    }

    pub(crate) fn contains_request(&self, id: u64) -> bool {
        self.requests.iter().any(|(open, _)| *open == id)
    }

    pub(crate) fn close(&mut self, id: u64) -> bool {
        if let Some(index) = self.requests.iter().position(|(open, _)| *open == id) {
            let (_, key) = self.requests.remove(index);
            self.open.retain(|open| open.key != key);
        }
        self.requests.is_empty()
    }

    /// 记下新打开的一条：换了房间时前一个房间的都关掉；同一个房间里换了旁聊时前一个旁聊关掉，主对话留着。
    pub(crate) fn insert(&mut self, timeline: OpenTimeline) {
        let key = timeline.key.clone();
        self.open.retain(|open| {
            open.key.room_id == key.room_id
                && open.key != key
                && (open.key.thread_root.is_none() || key.thread_root.is_none())
        });
        self.open.push(timeline);
    }

    pub(crate) fn get(&self, key: &TimelineKey) -> Option<&OpenTimeline> {
        self.open.iter().find(|open| &open.key == key)
    }

    /// 这个房间打开着的所有时间线，旁聊在前（刚点进去的那条）。
    pub(crate) fn all_in(&self, room_id: &OwnedRoomId) -> Vec<Arc<ChatRoom>> {
        let mut rooms: Vec<&OpenTimeline> = self
            .open
            .iter()
            .filter(|open| &open.key.room_id == room_id)
            .collect();
        rooms.sort_by_key(|open| open.key.thread_root.is_none());
        rooms.into_iter().map(|open| open.room.clone()).collect()
    }

    pub(crate) fn clear(&mut self) {
        self.open.clear();
        self.requests.clear();
    }
}

/// 从发起打开到订阅取消共用同一个编号，旧订阅迟到的关闭不能关掉新订阅。
pub(crate) struct TimelineOpening {
    pub(crate) key: TimelineKey,
    pub(crate) id: u64,
    pub(crate) chat: Weak<super::ChatLink>,
}

impl TimelineOpening {
    pub(crate) fn publish_panel(&self, panel: wonton_protocol::RoomPanel) -> bool {
        let Some(chat) = self.chat.upgrade() else {
            return false;
        };
        let open = super::lock(&chat.open);
        if !open.contains_request(self.id) {
            return false;
        }
        chat.room_panel_out
            .send_replace(Some((self.key.room_id.clone(), panel)));
        true
    }
}

impl Drop for TimelineOpening {
    fn drop(&mut self) {
        if let Some(chat) = self.chat.upgrade() {
            let mut open = super::lock(&chat.open);
            if open.close(self.id) {
                chat.room_panel_out.send_replace(None);
            }
        }
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
