//! 私聊：建私聊、按 `m.direct` 认已有的私聊和对方。

use matrix_sdk::{
    Room, RoomState,
    ruma::{
        OwnedRoomId, RoomId, UserId,
        api::{
            client::room::create_room::v3::{Request as CreateRoomRequest, RoomPreset},
            error::ErrorKind,
        },
        events::direct::DirectEventContent,
    },
};

use crate::{Error, MatrixSession, Result};

impl MatrixSession {
    pub async fn create_direct_chat(&self, peer: &UserId, encrypted: bool) -> Result<OwnedRoomId> {
        // 建房时邀请失败，服务器只记日志：对方不存在时房间照样建出来，里面只有自己（Tuwunel 上实测）。
        if let Err(error) = self.client.account().fetch_user_profile_of(peer).await
            && error.client_api_error_kind() == Some(&ErrorKind::NotFound)
        {
            return Err(Error::InvalidInput(format!("找不到用户 {peer}")));
        }
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
    pub fn direct_chat_with(&self, peer: &UserId) -> Option<OwnedRoomId> {
        self.client
            .get_dm_rooms(peer)
            .find(|room| matches!(room.state(), RoomState::Joined | RoomState::Invited))
            .map(|room| room.room_id().to_owned())
    }
    pub async fn direct_chat_rooms_with(&self, peer: &UserId) -> Result<Vec<OwnedRoomId>> {
        let raw = self
            .client
            .account()
            .account_data::<DirectEventContent>()
            .await
            .map_err(|error| Error::Upstream(error.to_string()))?;
        let Some(raw) = raw else {
            return Ok(Vec::new());
        };
        let event = raw
            .deserialize()
            .map_err(|error| Error::Upstream(error.to_string()))?;
        Ok(event
            .0
            .iter()
            .find(|(user, _)| user.as_user_id() == Some(peer))
            .map(|(_, rooms)| rooms.clone())
            .unwrap_or_default())
    }
}

pub(super) async fn direct_peer(room: &Room) -> Option<OwnedUserId> {
    match room.invite_details().await {
        Ok(invite) => Some(invite.inviter_id),
        Err(error) => {
            tracing::warn!(room = %room.room_id(), "读不到邀请人：{error}");
            None
        }
    }
}
