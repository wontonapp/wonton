//! 房间列表：建私聊、加入房间、观察房间及成员状态的变化。

use futures_util::{Stream, StreamExt, future, stream};
use matrix_sdk::{
    Client, RoomState,
    ruma::{
        OwnedRoomId, RoomId, UserId,
        api::client::room::create_room::v3::{Request as CreateRoomRequest, RoomPreset},
    },
};
use tokio::sync::broadcast::error::RecvError;

use crate::{ChatRoom, Error, MatrixSession, Result};

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
pub struct RoomSummary {
    /// 房间 ID。
    pub room_id: OwnedRoomId,
    /// 本账号的成员状态。
    pub membership: Membership,
}

impl MatrixSession {
    /// 和另一个用户建一个私聊房间并邀请对方，返回房间 ID。
    ///
    /// 房间不开端到端加密：是否加密取决于推送方案，见 ADR-0003 的验证结果。
    pub async fn create_direct_chat(&self, peer: &UserId) -> Result<OwnedRoomId> {
        let mut request = CreateRoomRequest::new();
        request.invite = vec![peer.to_owned()];
        request.is_direct = true;
        request.preset = Some(RoomPreset::TrustedPrivateChat);
        let room = self.client.create_room(request).await?;
        Ok(room.room_id().to_owned())
    }

    /// 加入房间，用于接受邀请。
    pub async fn join_room(&self, room_id: &RoomId) -> Result<()> {
        self.client.join_room_by_id(room_id).await?;
        Ok(())
    }

    /// 房间列表流：第一项是当前列表，之后每次同步到房间变化给出新列表。
    pub fn room_updates(&self) -> impl Stream<Item = Vec<RoomSummary>> + use<> {
        let client = self.client.clone();
        let current = summaries(&client);
        let changes = stream::unfold(
            client.subscribe_to_all_room_updates(),
            |mut receiver| async {
                match receiver.recv().await {
                    Ok(_) | Err(RecvError::Lagged(_)) => Some(((), receiver)),
                    Err(RecvError::Closed) => None,
                }
            },
        );
        stream::once(future::ready(current)).chain(changes.map(move |()| summaries(&client)))
    }

    /// 打开一个已加入的房间，返回可以收发消息的 [`ChatRoom`]。
    ///
    /// 房间列表只同步每个房间的最新一条消息；打开房间时才订阅它的完整时间线，和在手机上点进
    /// 一个聊天一致。同一时间只订阅一个房间，打开另一个房间会取消前一个的订阅。
    pub async fn open_room(&self, room_id: &RoomId) -> Result<ChatRoom> {
        let room = self
            .client
            .get_room(room_id)
            .ok_or_else(|| Error::NotReady(format!("房间 {room_id} 还没有同步到本地")))?;
        if room.state() != RoomState::Joined {
            return Err(Error::NotReady(format!("还没有加入房间 {room_id}")));
        }
        self.sync
            .room_list_service()
            .set_room_subscriptions(&[room_id])
            .await;
        ChatRoom::open(room).await
    }
}

fn summaries(client: &Client) -> Vec<RoomSummary> {
    client
        .rooms()
        .into_iter()
        .map(|room| RoomSummary {
            room_id: room.room_id().to_owned(),
            membership: room.state().into(),
        })
        .collect()
}
