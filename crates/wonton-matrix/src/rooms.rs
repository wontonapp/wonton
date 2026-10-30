//! 房间列表：按最近活动排序，带名字、最新消息预览和未读数；建私聊、加入和离开房间。
//!
//! 列表由 matrix-sdk-ui 的 `RoomListService` 维护（和 Element X 一样），排序也用它的：正在发送的在前，
//! 其余按最近活动，再按名字。

use std::pin::pin;

use futures_util::{Stream, StreamExt, stream};
use matrix_sdk::{
    Room, RoomMemberships, RoomState,
    ruma::{
        MilliSecondsSinceUnixEpoch, OwnedRoomId, OwnedUserId, RoomId, UserId,
        api::client::room::create_room::v3::{Request as CreateRoomRequest, RoomPreset},
        events::{GlobalAccountDataEvent, direct::DirectEventContent},
    },
};
use matrix_sdk_ui::{
    eyeball_im::Vector,
    room_list_service::{RoomListItem, filters},
    timeline::{LatestEventValue, Profile, RoomExt, TimelineDetails, TimelineItemContent},
};
use tokio::sync::watch;

use crate::{ChatRoom, Error, MatrixSession, MessageBody, Result, message::body};

/// 房间列表一次最多展示的房间数。家里服务器的房间不多，一页就够。
const PAGE_SIZE: usize = 500;

/// 本账号在房间里的成员状态。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Membership {
    /// 已加入。
    Joined,
    /// 收到邀请，还没加入。
    Invited,
    /// 已离开。
    Left,
    /// 已申请加入，等待批准。
    Knocked,
    /// 被封禁。
    Banned,
}

impl From<RoomState> for Membership {
    fn from(state: RoomState) -> Self {
        match state {
            RoomState::Joined => Self::Joined,
            RoomState::Invited => Self::Invited,
            RoomState::Left => Self::Left,
            RoomState::Knocked => Self::Knocked,
            RoomState::Banned => Self::Banned,
        }
    }
}

/// 房间列表里的一项。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoomInfo {
    /// 房间 ID。
    pub room_id: OwnedRoomId,
    /// 显示名：房间名，私聊时是对方的名字。
    pub name: String,
    /// 是否是私聊。
    pub is_direct: bool,
    /// 私聊对方：加入了的私聊是 `m.direct` 里唯一的对方，和 [`MatrixSession::direct_chat_with`] 认私聊的办法一样；
    /// 受邀还没加入的私聊是邀请人。
    pub direct_user_id: Option<OwnedUserId>,
    /// 是否开了端到端加密。
    pub is_encrypted: bool,
    /// 本账号的成员状态。
    pub membership: Membership,
    /// 最新一条消息；还没有消息或者还没算出来时为 `None`。
    pub latest: Option<LatestMessage>,
    /// 未读消息数，按本账号的已读回执在本机算出。
    pub unread_count: u64,
}

/// 房间的最新一条消息。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LatestMessage {
    /// 发送者的显示名。
    pub sender_name: String,
    /// 是否由本账号发出。
    pub is_own: bool,
    /// 内容；不是消息的事件（成员变化等）是 [`MessageBody::Other`]。
    pub body: MessageBody,
    /// 发送时间，Unix 毫秒。
    pub timestamp_ms: u64,
}

impl MatrixSession {
    /// 和另一个用户建一个私聊房间并邀请对方，返回房间 ID。`encrypted` 决定是否开端到端加密。
    pub async fn create_direct_chat(&self, peer: &UserId, encrypted: bool) -> Result<OwnedRoomId> {
        let room = if encrypted {
            self.client.create_dm(peer).await?
        } else {
            let mut request = CreateRoomRequest::new();
            request.invite = vec![peer.to_owned()];
            request.is_direct = true;
            request.preset = Some(RoomPreset::TrustedPrivateChat);
            self.client.create_room(request).await?
        };
        Ok(room.room_id().to_owned())
    }

    /// 已有的和 `peer` 的私聊（已加入或已受邀的）。
    pub fn direct_chat_with(&self, peer: &UserId) -> Option<OwnedRoomId> {
        self.client
            .get_dm_rooms(peer)
            .find(|room| matches!(room.state(), RoomState::Joined | RoomState::Invited))
            .map(|room| room.room_id().to_owned())
    }

    /// 加入房间，用于接受邀请。私聊邀请加入后记进 `m.direct`，之后列表里仍标为私聊。
    pub async fn join_room(&self, room_id: &RoomId) -> Result<()> {
        // 私聊标记要在加入前从邀请里读。SDK 也会记 `m.direct`，但它在加入请求返回后才检查房间是否还是
        // “受邀”，同步抢先处理了加入时就漏记了（同步一直在跑，实测负载高时常发生）。
        let was_direct = match self.client.get_room(room_id) {
            Some(room) if room.state() == RoomState::Invited => {
                room.is_direct().await.unwrap_or_else(|error| {
                    tracing::warn!(room = %room_id, "读不到邀请里的私聊标记：{error}");
                    false
                })
            }
            _ => false,
        };
        let room = self.client.join_room_by_id(room_id).await?;
        if was_direct && room.direct_targets_length() == 0 {
            room.set_is_direct(true).await?;
        }
        Ok(())
    }

    /// 拒绝邀请，或者离开已加入的房间。
    pub async fn leave_room(&self, room_id: &RoomId) -> Result<()> {
        self.room(room_id)?.leave().await?;
        Ok(())
    }

    /// 房间列表流：第一项是算出来的第一份列表（SDK 设好过滤器后总会先给一份，没有房间时是空表），之后列表或
    /// 其中的房间变化时给出新列表；只含已加入和收到邀请的房间。
    ///
    /// 来不及取走的中间列表会被合并掉。要先 [`start_sync`](Self::start_sync) 才会有房间。
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

    /// 打开一个已加入的房间，返回可以收发消息的 [`ChatRoom`]。
    ///
    /// 房间列表只同步每个房间的最新一条消息；打开房间时才订阅它的完整时间线，和在手机上点进
    /// 一个聊天一致。同一时间只订阅一个房间，打开另一个房间会取消前一个的订阅。
    pub async fn open_room(&self, room_id: &RoomId) -> Result<ChatRoom> {
        let room = self.room(room_id)?;
        if room.state() != RoomState::Joined {
            return Err(Error::NotReady(format!("还没有加入房间 {room_id}")));
        }
        self.sync
            .room_list_service()
            .set_room_subscriptions(&[room_id])
            .await;
        ChatRoom::open(room).await
    }

    fn room(&self, room_id: &RoomId) -> Result<Room> {
        self.client
            .get_room(room_id)
            .ok_or_else(|| Error::NotReady(format!("房间 {room_id} 还没有同步到本地")))
    }
}

/// 把 watch 通道变成流：第一项是第一份算出来的值（还没算出来就等），之后每次变化给出最新值。
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
    // 收到邀请时私聊标记在邀请事件里，加入后才进 `m.direct`；SDK 的 `is_direct` 两种都处理。
    let is_direct = room.is_direct().await.unwrap_or_else(|error| {
        tracing::warn!(room = %room.room_id(), "读不到私聊标记，按群聊显示：{error}");
        false
    });
    let mut name = match room.cached_display_name() {
        Some(name) => name.to_string(),
        None => room
            .display_name()
            .await
            .map_or_else(|_| room.room_id().to_string(), |name| name.to_string()),
    };
    // Tuwunel 的 Sliding Sync 不给房间的 heroes 带显示名，SDK 只能拿用户 ID 当私聊的名字；这时去本机的成员
    // 事件里找对方的显示名（邀请里也有邀请人的成员事件）。
    if is_direct
        && name.starts_with('@')
        && let Some(peer) = peer_name(room).await
    {
        name = peer;
    }
    RoomInfo {
        room_id: room.room_id().to_owned(),
        name,
        is_direct,
        direct_user_id: if is_direct {
            direct_peer(room).await
        } else {
            None
        },
        is_encrypted: room.encryption_state().is_encrypted(),
        membership: room.state().into(),
        latest: latest_message(room).await,
        unread_count: room.num_unread_messages(),
    }
}

/// 受邀的房间还没进 `m.direct`，对方取邀请人。
async fn direct_peer(room: &Room) -> Option<OwnedUserId> {
    if room.state() == RoomState::Invited {
        return match room.invite_details().await {
            Ok(invite) => Some(invite.inviter_id),
            Err(error) => {
                tracing::warn!(room = %room.room_id(), "读不到邀请人：{error}");
                None
            }
        };
    }
    let targets = room.direct_targets();
    if targets.len() != 1 {
        return None;
    }
    targets.iter().next()?.as_user_id().map(ToOwned::to_owned)
}

/// 私聊里对方的显示名；没有设置或者本机还没有对方的成员事件时为 `None`。
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
    Some(LatestMessage {
        sender_name: profile_name(&profile, &sender),
        is_own,
        body: match &content {
            TimelineItemContent::MsgLike(message) => body(&message.kind),
            _ => MessageBody::Other {
                kind: "state".to_owned(),
            },
        },
        timestamp_ms: millis(timestamp),
    })
}

fn profile_name(profile: &TimelineDetails<Profile>, sender: &OwnedUserId) -> String {
    match profile {
        TimelineDetails::Ready(Profile {
            display_name: Some(name),
            ..
        }) => name.clone(),
        _ => sender.to_string(),
    }
}

fn millis(timestamp: MilliSecondsSinceUnixEpoch) -> u64 {
    timestamp.get().into()
}
