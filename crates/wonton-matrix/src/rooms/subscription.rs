//! Sliding Sync 的房间订阅：打开、常驻 watch、独立时间线。

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use matrix_sdk::{
    Room,
    ruma::{OwnedRoomId, RoomId},
};

use crate::{ChatRoom, MatrixSession, Result};

#[derive(Default)]
pub(crate) struct RoomSubscriptions {
    /// 打开着的房间。
    opened: Option<OwnedRoomId>,
    /// 桥等调用方显式打开、关闭的常驻 watch；重复打开不增加引用。
    watched: BTreeSet<OwnedRoomId>,
    /// 每份独立主对话或旁聊由 ChatRoom 的存活引用计数。
    focused: BTreeMap<OwnedRoomId, usize>,
    /// 加入后同步还标着“受邀”、临时订阅的房间，见 [`MatrixSession::until_joined`]。
    pub(super) joining: BTreeSet<OwnedRoomId>,
    /// 上一次发给 SDK 的并集。名单没变就不发：SDK 收到订阅变化会取消在飞的同步请求，一次名单没变的
    /// 重发是无害的浪费，一次名单真变大的重发在 Tuwunel 上能把别的在飞请求顶成 `M_UNKNOWN_POS`（#370）。
    applied: BTreeSet<OwnedRoomId>,
}

pub(crate) struct RoomWatch {
    session: Arc<MatrixSession>,
    room_id: OwnedRoomId,
    runtime: tokio::runtime::Handle,
}

impl Drop for RoomWatch {
    fn drop(&mut self) {
        let session = self.session.clone();
        let room_id = self.room_id.clone();
        self.runtime.spawn(async move {
            session.release_focused(&room_id).await;
        });
    }
}

impl MatrixSession {
    pub async fn open_room(&self, room_id: &RoomId) -> Result<ChatRoom> {
        let room = self.subscribe_opened(room_id).await?;
        ChatRoom::open(room).await
    }
    pub(crate) async fn subscribe_opened(&self, room_id: &RoomId) -> Result<Room> {
        let room = self.joined(room_id)?;
        let mut subscriptions = self.subscriptions.lock().await;
        subscriptions.opened = Some(room_id.to_owned());
        self.subscribe(&mut subscriptions, false).await;
        Ok(room)
    }
    pub async fn watch_room(&self, room_id: &RoomId) -> Result<ChatRoom> {
        let room = self.joined(room_id)?;
        let mut subscriptions = self.subscriptions.lock().await;
        subscriptions.watched.insert(room_id.to_owned());
        self.subscribe(&mut subscriptions, false).await;
        drop(subscriptions);
        ChatRoom::open(room).await
    }
    pub(crate) async fn subscribe_watched(
        self: &Arc<Self>,
        room_id: &RoomId,
    ) -> Result<(Room, RoomWatch)> {
        let room = self.joined(room_id)?;
        let mut subscriptions = self.subscriptions.lock().await;
        *subscriptions.focused.entry(room_id.to_owned()).or_default() += 1;
        let watch = RoomWatch {
            session: self.clone(),
            room_id: room_id.to_owned(),
            runtime: tokio::runtime::Handle::current(),
        };
        self.subscribe(&mut subscriptions, false).await;
        Ok((room, watch))
    }
    pub async fn unwatch_room(&self, room_id: &RoomId) {
        let mut subscriptions = self.subscriptions.lock().await;
        subscriptions.watched.remove(room_id);
        self.subscribe(&mut subscriptions, false).await;
    }
    async fn release_focused(&self, room_id: &RoomId) {
        let mut subscriptions = self.subscriptions.lock().await;
        if let Some(count) = subscriptions.focused.get_mut(room_id) {
            *count -= 1;
            if *count == 0 {
                subscriptions.focused.remove(room_id);
            }
        }
        self.subscribe(&mut subscriptions, false).await;
    }
    pub(super) async fn subscribe(
        &self,
        subscriptions: &mut RoomSubscriptions,
        force_redeliver: bool,
    ) {
        let rooms: BTreeSet<OwnedRoomId> = subscriptions
            .opened
            .iter()
            .chain(&subscriptions.watched)
            .chain(subscriptions.focused.keys())
            .chain(&subscriptions.joining)
            .cloned()
            .collect();
        if rooms == subscriptions.applied && !force_redeliver {
            return;
        }
        let list: Vec<&RoomId> = rooms.iter().map(|room| &**room).collect();
        if force_redeliver {
            self.sync
                .room_list_service()
                .reset_and_add_room_subscriptions(&list)
                .await;
        } else {
            self.sync
                .room_list_service()
                .set_room_subscriptions(&list)
                .await;
        }
        subscriptions.applied = rooms;
    }
    pub(crate) async fn resubscribe(&self, subscriptions: &mut RoomSubscriptions) {
        subscriptions.applied.clear();
        self.subscribe(subscriptions, false).await;
    }
}
