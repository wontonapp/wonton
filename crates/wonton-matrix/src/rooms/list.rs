//! 房间列表流和列表行：名字、私聊对方、最新消息、未读数。

use std::pin::pin;

use futures_util::{Stream, StreamExt, stream};
use matrix_sdk::{
    Room, RoomDisplayName, RoomMemberships, RoomState,
    ruma::{
        MilliSecondsSinceUnixEpoch, OwnedUserId,
        events::{GlobalAccountDataEvent, direct::DirectEventContent},
    },
};
use matrix_sdk_ui::{
    eyeball_im::Vector,
    room_list_service::{RoomListItem, filters},
    timeline::{LatestEventValue, Profile, RoomExt, TimelineDetails, TimelineItemContent},
};
use tokio::sync::watch;

use super::{LatestMessage, RoomInfo, direct::direct_peer};
use crate::{Error, MatrixSession, MessageBody, Result, message::body};

const PAGE_SIZE: usize = 500;

impl MatrixSession {
    pub async fn rooms(&self) -> Result<impl Stream<Item = Vec<RoomInfo>> + use<>> {
        let list = self
            .sync
            .room_list_service()
            .all_rooms()
            .await
            .map_err(|error| Error::Upstream(error.to_string()))?;
        // SDK 处理 `m.direct` 时不算房间的显著变化，列表不会因此重发这个房间；单独观察它，私聊标记变了也重算。
        let direct_changes = self
            .client
            .observe_events::<GlobalAccountDataEvent<DirectEventContent>, ()>();
        // 房间的未读加上各旁聊的未读（ADR-0093 §4）：话题里来了消息只动事件缓存，话题回执只动话题，SDK 都不当成
        // 房间的变化，单独跟着。
        let mut cache_changes = self
            .client
            .event_cache()
            .subscribe_to_room_generic_updates();
        let rooms_changed = self.rooms_changed.clone();
        // 初值是“还没算出来”，不是空列表：界面要分得清“还在加载”和“真的没有房间”。
        let (sender, receiver) = watch::channel(None);
        tokio::spawn(async move {
            let (entries, controller) = list.entries_with_dynamic_adapters(PAGE_SIZE);
            controller.set_filter(Box::new(filters::new_filter_non_left()));
            let mut entries = pin!(entries);
            let mut direct_changes = direct_changes.subscribe();
            let mut rooms = Vector::<RoomListItem>::new();
            loop {
                tokio::select! {
                    batch = entries.next() => {
                        let Some(batch) = batch else { return };
                        for diff in batch {
                            diff.apply(&mut rooms);
                        }
                    }
                    Some(_) = direct_changes.next() => {}
                    Ok(_) = cache_changes.recv() => {}
                    () = rooms_changed.notified() => {}
                    () = sender.closed() => return,
                }
                let mut infos = Vec::with_capacity(rooms.len());
                for room in &rooms {
                    infos.push(room_info(room).await);
                }
                if sender.send(Some(infos)).is_err() {
                    return;
                }
            }
        });
        Ok(snapshots(receiver))
    }
    pub fn refresh_rooms(&self) {
        self.rooms_changed.notify_one();
    }
}

fn snapshots<T: Clone + Send + Sync + 'static>(
    receiver: watch::Receiver<Option<T>>,
) -> impl Stream<Item = T> {
    stream::unfold((receiver, true), |(mut receiver, first)| async move {
        if !first {
            receiver.changed().await.ok()?;
        }
        loop {
            let value = receiver.borrow_and_update().clone();
            if let Some(value) = value {
                return Some((value, (receiver, false)));
            }
            receiver.changed().await.ok()?;
        }
    })
}

async fn room_info(room: &Room) -> RoomInfo {
    // 各字段隔着 await 取，成员状态可能在中间变化（比如接受邀请的加入在这时落地），拼出来的行会
    // 自相矛盾：`is_direct` 按受邀时邀请里的标记算，`direct_user_id` 却按加入后还没同步回来的
    // `m.direct` 算，得到「已加入、是私聊、不知道对方是谁」。以读到的状态为准，拼完核对，变了就按
    // 新状态重拼；成员状态变化本来就会让房间列表重发。
    let mut state = room.state();
    loop {
        let info = room_info_in(room, state).await;
        if room.state() == state {
            return info;
        }
        state = room.state();
    }
}

async fn room_info_in(room: &Room, state: RoomState) -> RoomInfo {
    // 私聊标记和对方从同一份数据取：好友邀请不一定能从邀请 stripped state 恢复 `is_direct`，但好友标记仍应进请求列表。
    let (is_direct, direct_user_id) = if state == RoomState::Invited {
        let friend = crate::friends::marked_friend(room).await;
        let is_direct = friend
            || room.is_direct().await.unwrap_or_else(|error| {
                tracing::warn!(room = %room.room_id(), "读不到私聊标记，按群聊显示：{error}");
                false
            });
        let peer = if is_direct {
            direct_peer(room).await
        } else {
            None
        };
        (is_direct, peer)
    } else {
        let targets = room.direct_targets();
        let is_direct = !targets.is_empty() || crate::friends::marked_friend(room).await;
        let peer = if !targets.is_empty() && targets.len() == 1 {
            targets
                .iter()
                .next()
                .and_then(|target| target.as_user_id().map(ToOwned::to_owned))
        } else {
            None
        };
        (is_direct, peer)
    };
    let mut name = match room.cached_display_name() {
        Some(name) => name.to_string(),
        None => room
            .display_name()
            .await
            .map_or_else(|_| room.room_id().to_string(), |name| name.to_string()),
    };
    // Tuwunel 的 Sliding Sync 不给房间的 heroes 带显示名，SDK 只能拿用户 ID 当私聊的名字；成员事件
    // 到得比显示名计算晚时，SDK 还会把 "Empty Room" 这类占位名缓存住，没有新的房间更新就不重算。
    // 这些占位名都不可读，去本机的成员事件里找对方的显示名（邀请里也有邀请人的成员事件）。
    if is_direct
        && (name.starts_with('@')
            || matches!(
                room.cached_display_name(),
                Some(RoomDisplayName::Empty | RoomDisplayName::EmptyWas(_))
            ))
        && let Some(peer) = peer_name(room).await
    {
        name = peer;
    }
    RoomInfo {
        room_id: room.room_id().to_owned(),
        name,
        is_direct,
        is_friend: crate::friends::marked_friend(room).await,
        direct_user_id,
        is_encrypted: room.encryption_state().is_encrypted(),
        membership: state.into(),
        latest: latest_message(room).await,
        unread_count: room.num_unread_messages() + crate::side_chat::threads_unread(room).await,
        unread_mentions: room.num_unread_mentions(),
        call_active: room.has_active_room_call(),
    }
}

async fn peer_name(room: &Room) -> Option<String> {
    let members = room
        .members_no_sync(RoomMemberships::JOIN | RoomMemberships::INVITE)
        .await
        .ok()?;
    let own = room.own_user_id();
    members
        .iter()
        .find(|member| member.user_id() != own)
        .and_then(|member| member.display_name().map(ToOwned::to_owned))
}

async fn latest_message(room: &Room) -> Option<LatestMessage> {
    let (timestamp, sender, is_own, profile, content) = match RoomExt::latest_event(room).await {
        LatestEventValue::Remote {
            timestamp,
            sender,
            is_own,
            profile,
            content,
        } => (timestamp, sender, is_own, profile, content),
        LatestEventValue::Local {
            timestamp,
            sender,
            profile,
            content,
            ..
        } => (timestamp, sender, true, profile, content),
        LatestEventValue::RemoteInvite { .. } | LatestEventValue::None => return None,
    };
    let sender_name = profile_name(&profile, &sender);
    Some(LatestMessage {
        sender_id: sender,
        sender_name,
        is_own,
        body: match &content {
            TimelineItemContent::MsgLike(message) => body(&message.kind),
            _ => MessageBody::Other {
                kind: "state".to_owned(),
            },
        },
        timestamp_ms: millis(timestamp),
        side_chat: crate::side_chat::latest_side_chat(room).await,
    })
}

fn profile_name(profile: &TimelineDetails<Profile>, sender: &OwnedUserId) -> String {
    match profile {
        TimelineDetails::Ready(Profile {
            display_name: Some(name),
            ..
        }) if !name.trim().is_empty() => name.clone(),
        _ => crate::message::fallback_name(sender).to_owned(),
    }
}

fn millis(timestamp: MilliSecondsSinceUnixEpoch) -> u64 {
    timestamp.get().into()
}
