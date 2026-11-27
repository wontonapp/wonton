//! 房间列表：按最近活动排序，带名字、最新消息预览和未读数；建私聊、加入和离开房间。
//!
//! 列表由 matrix-sdk-ui 的 `RoomListService` 维护（和 Element X 一样），排序也用它的：正在发送的在前，
//! 其余按最近活动，再按名字。

use crate::Error;
use crate::MatrixSession;
use crate::MessageBody;
use crate::Result;
use matrix_sdk::Room;
use matrix_sdk::RoomState;
use matrix_sdk::ruma::OwnedRoomId;
use matrix_sdk::ruma::OwnedUserId;
use matrix_sdk::ruma::RoomId;

mod direct;
mod draft;
mod join;
mod list;
mod send;
mod subscription;

pub(crate) use subscription::{RoomSubscriptions, RoomWatch};

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
    /// 建房时带了好友标记（ADR-0051）：邀请是一条好友请求，加入后是好友私聊。
    pub is_friend: bool,
    /// 最新一条消息；还没有消息或者还没算出来时为 `None`。
    pub latest: Option<LatestMessage>,
    /// 未读消息数，按本账号的已读回执在本机算出。
    pub unread_count: u64,
    /// 未读里提到本账号的条数（ADR-0121 的「@」徽标）；话题里的提及也算在房间头上。
    pub unread_mentions: u64,
    /// 房间里有人在通话，同 [`ChatRoom::call_active`](crate::ChatRoom::call_active)。
    pub call_active: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LatestMessage {
    /// 发送者的 Matrix 用户 ID。
    pub sender_id: OwnedUserId,
    /// 发送者的显示名。
    pub sender_name: String,
    /// 是否由本账号发出。
    pub is_own: bool,
    /// 内容；不是消息的事件（成员变化等）是 [`MessageBody::Other`]。
    pub body: MessageBody,
    /// 发送时间，Unix 毫秒。
    pub timestamp_ms: u64,
    /// 这一条属于旁聊（根、结果行、话题里的消息）时，旁聊的标题和结果行（ADR-0093 §4）。
    pub side_chat: Option<crate::side_chat::LatestSideChat>,
}

impl MatrixSession {
    pub(crate) fn joined(&self, room_id: &RoomId) -> Result<Room> {
        let room = self.room(room_id)?;
        if room.state() != RoomState::Joined {
            return Err(Error::NotReady("还没有加入这个聊天".to_owned()));
        }
        Ok(room)
    }
    pub(crate) fn room(&self, room_id: &RoomId) -> Result<Room> {
        self.client.get_room(room_id).ok_or_else(|| {
            Error::NotReady("这个聊天还没有同步到这台设备，稍等一会儿再试".to_owned())
        })
    }
}
