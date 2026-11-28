//! 加入、离开房间：解房间链接、接受邀请、等同步标成已加入。

use std::time::Duration;

use matrix_sdk::{
    Room, RoomState,
    ruma::{OwnedRoomId, RoomId, api::client::membership::join_room_by_id},
};

use crate::{Error, MatrixSession, Result};

const INVITE_WAIT: Duration = Duration::from_secs(10);

pub(super) const ROOM_SYNC_WAIT: Duration = Duration::from_secs(20);

const JOIN_ECHO_WAIT: Duration = Duration::from_secs(3);

const RESUBSCRIBED_WAIT: Duration = Duration::from_secs(30);

impl MatrixSession {
    pub async fn resolve_room_id(&self, input: &str) -> Result<OwnedRoomId> {
        let target = <&matrix_sdk::ruma::RoomOrAliasId>::try_from(input)
            .map_err(|error| Error::InvalidInput(error.to_string()))?;
        match <&RoomId>::try_from(target) {
            Ok(id) => Ok(id.to_owned()),
            Err(alias) => Ok(self.client.resolve_room_alias(alias).await?.room_id),
        }
    }
    pub async fn join_room(&self, room_id: &RoomId) -> Result<()> {
        // 私聊标记和邀请人要在加入前从邀请里读。不用 SDK 的 `join_room_by_id`：它加入后用 `set_is_direct`
        // 记私聊，拿本机存的 `m.direct` 改完整份写回，而本机这份要等写入同步回来才更新。一次同步之内先后接受
        // 两个私聊邀请时，后一次写的没有前一个房间，前一个就不再是私聊（通知房间因此能发消息，负载高时实测）。
        // `mark_as_dm` 先从服务器取最新的再改，同一会话里的写入有锁串着。代价是不导入邀请人随邀请分享的历史
        // 密钥：SDK 只在单独邀请时分享，建房时一起发的私聊邀请本来就没有。
        let invited = self
            .client
            .get_room(room_id)
            .filter(|room| room.state() == RoomState::Invited);
        // 好友邀请也记成私聊：它的邀请请求本身不带 `is_direct`，靠建房时的标记认。
        let inviter = match &invited {
            Some(room)
                if invited_as_direct(room).await || crate::friends::marked_friend(room).await =>
            {
                Some(room.invite_details().await?.inviter_id)
            }
            _ => None,
        };
        self.client
            .send(join_room_by_id::v3::Request::new(room_id.to_owned()))
            .await?;
        if let Some(inviter) = inviter {
            self.client
                .account()
                .mark_as_dm(room_id, &[inviter])
                .await?;
        }
        match invited {
            Some(room) => self.until_joined(&room).await,
            // 邀请还没同步到本地：房间可能还没进本地库，按 [`Self::joined_within`] 等它同步到并成为已加入。
            None => self
                .joined_within(room_id, ROOM_SYNC_WAIT)
                .await
                .map(|_| ()),
        }
    }
    async fn until_joined(&self, room: &Room) -> Result<()> {
        if joined(room, JOIN_ECHO_WAIT).await {
            return Ok(());
        }
        tracing::warn!(room = %room.room_id(), "加入后同步仍标着受邀，重新订阅这个房间");
        let room_id = room.room_id().to_owned();
        let mut subscriptions = self.subscriptions.lock().await;
        subscriptions.joining.insert(room_id.clone());
        // 这次发送的目的就是让服务器按当前状态重发这个房间：房间多半还在名单里（跟着卡片），名单没变的
        // 下发 SDK 只当空操作，不会取消在飞的请求，服务器也就不会重发，所以强制。
        self.subscribe(&mut subscriptions, true).await;
        drop(subscriptions);
        let done = joined(room, RESUBSCRIBED_WAIT).await;
        let mut subscriptions = self.subscriptions.lock().await;
        subscriptions.joining.remove(&room_id);
        self.subscribe(&mut subscriptions, false).await;
        drop(subscriptions);
        if done {
            Ok(())
        } else {
            Err(Error::NotReady(
                "已经加入，但服务器一直没把这个聊天同步过来".to_owned(),
            ))
        }
    }
    pub async fn leave_room(&self, room_id: &RoomId) -> Result<()> {
        self.room(room_id)?.leave().await?;
        Ok(())
    }
    pub(super) async fn joined_within(&self, room_id: &RoomId, wait: Duration) -> Result<Room> {
        let deadline = tokio::time::Instant::now() + wait;
        loop {
            if let Some(room) = self.client.get_room(room_id)
                && room.state() == RoomState::Joined
            {
                return Ok(room);
            }
            if tokio::time::Instant::now() >= deadline {
                return self.joined(room_id);
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }
}

async fn joined(room: &Room, wait: Duration) -> bool {
    let mut updates = room.subscribe_info();
    let joined = async {
        while room.state() != RoomState::Joined {
            if updates.next().await.is_none() {
                return false;
            }
        }
        true
    };
    tokio::time::timeout(wait, joined).await.unwrap_or(false)
}

async fn invited_as_direct(room: &Room) -> bool {
    let mut updates = room.subscribe_info();
    let invite_stored = async {
        loop {
            match room.get_member_no_sync(room.own_user_id()).await {
                Ok(None) => {}
                other => return other.map(|_| ()),
            }
            updates.next().await;
        }
    };
    let direct = match tokio::time::timeout(INVITE_WAIT, invite_stored).await {
        Ok(Ok(())) => room.is_direct().await.map_err(matrix_sdk::Error::from),
        Ok(Err(error)) => Err(error),
        Err(_) => {
            tracing::warn!(room = %room.room_id(), "等不到邀请事件，按群聊加入");
            Ok(false)
        }
    };
    direct.unwrap_or_else(|error| {
        tracing::warn!(room = %room.room_id(), "读不到邀请里的私聊标记：{error}");
        false
    })
}
