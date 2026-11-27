//! 房间成员：取成员和显示名、@ 列表、发送前确认对方有设备。

use std::{collections::HashMap, sync::Arc, time::Duration};

use matrix_sdk::{
    Room, RoomMemberships,
    ruma::{OwnedUserId, events::room::message::RoomMessageEventContent},
};
use matrix_sdk_ui::timeline::Timeline;
use tokio::sync::{Notify, broadcast::error::RecvError, watch};

use super::{ChatRoom, MentionMember};
use crate::{
    Result,
    message::{MemberNames, RedactionPower},
};

/// 成员没取到时，隔多久再取一次：从这个间隔开始，每次加倍。
const FIRST_MEMBERS_RETRY: Duration = Duration::from_secs(2);
const MAX_MEMBERS_RETRY: Duration = Duration::from_secs(30);

impl ChatRoom {
    /// 现在这份成员和显示名。
    pub(crate) fn names(&self) -> Arc<MemberNames> {
        self.names.borrow().clone()
    }

    /// 文字事件：`users` 是过滤过的被提及者，写进 `m.mentions` 和正文的胶囊。
    pub(crate) fn mention_content(
        &self,
        body: &str,
        users: &[OwnedUserId],
    ) -> RoomMessageEventContent {
        let names = self.names();
        crate::mentions::content(body, users, &names)
    }

    /// 房间里已加入的成员数，只读本机数据，不等网络。
    pub async fn joined_member_count(&self) -> Result<usize> {
        Ok(self
            .room
            .members_no_sync(RoomMemberships::JOIN)
            .await?
            .len())
    }

    /// 在场（已加入）成员的用户 ID，只读本机数据，不等网络；成员还没同步到时可能不全。
    /// 跟着显示名的那份成员数据一起刷新。
    pub fn member_user_ids(&self) -> Vec<String> {
        self.names.borrow().user_ids()
    }

    /// 成员名单有没有从服务器取全；取 [`Self::member_user_ids`] 前先看这个，没取全时名单可能缺人。
    pub fn are_members_synced(&self) -> bool {
        self.room.are_members_synced()
    }

    /// 输入框 @ 列表的成员：在场的人（不含自己），带显示名和头像的 `mxc://`。私聊为空。
    pub fn mention_members(&self) -> Vec<MentionMember> {
        let names = self.names.borrow();
        if names.direct() {
            return Vec::new();
        }
        names
            .members()
            .filter(|(user, _)| !names.is_own(user))
            .map(|(user, name)| MentionMember {
                user_id: user.to_string(),
                display_name: name.to_owned(),
                avatar: names.avatar(user).map(str::to_owned),
            })
            .collect()
    }

    /// 成员或显示名变了（@ 列表跟着补发）。
    pub async fn member_names_changed(&self) {
        let mut names = self.names.clone();
        let _ = names.changed().await;
    }
}

/// 加密房间里发东西之前，先把对方的设备表查全。SDK 分享房间密钥时只等 5 秒 `/keys/query`（
/// `get_missing_sessions`），没回来就带着不完整的设备表往下走，可以把密钥分享给 0 个设备：事件照样发出去，
/// 收端永远解不开，也没有重试。这里对有成员没有设备的房间先查一次，回到本地已经有设备为止（每个房间只查缺的人，
/// 拿到以后再发不再查）。
pub(crate) async fn ensure_recipient_devices(room: &Room) -> Result<()> {
    if !room.latest_encryption_state().await?.is_encrypted() {
        return Ok(());
    }
    let encryption = room.client().encryption();
    for member in room.members_no_sync(RoomMemberships::JOIN).await? {
        let user = member.user_id();
        if user == room.own_user_id() {
            continue;
        }
        if encryption
            .get_user_devices(user)
            .await
            .is_ok_and(|devices| devices.devices().next().is_some())
        {
            continue;
        }
        // 查不到也照样发：分享密钥时 SDK 还会自己再等一次，别把离线时的发送挡在这里。
        encryption.request_user_identity(user).await.ok();
    }
    Ok(())
}

/// `fetch_members` 失败时只把错误记在时间线的条目上，不报告也不重试，要靠 `are_members_synced` 才知道取到没有。房间还没
/// 加入时它什么也不取，同样直接返回，也算没取到。
pub(super) async fn fetch_members_until_done(
    room: Room,
    timeline: Arc<Timeline>,
    fetched: Arc<Notify>,
) {
    let mut delay = FIRST_MEMBERS_RETRY;
    loop {
        timeline.fetch_members().await;
        fetched.notify_one();
        if room.are_members_synced() {
            return;
        }
        tracing::info!(room = %room.room_id(), ?delay, "成员没取到，稍后再取");
        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(MAX_MEMBERS_RETRY);
    }
}

pub(super) async fn refresh_member_names(
    room: Room,
    names: watch::Sender<Arc<MemberNames>>,
    fetched: Arc<Notify>,
) {
    let mut updates = room.subscribe_to_updates();
    loop {
        let fresh = local_member_names(&room).await;
        names.send_if_modified(|names| {
            if **names == fresh {
                return false;
            }
            *names = Arc::new(fresh);
            true
        });
        tokio::select! {
            update = updates.recv() => match update {
                Ok(_) | Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => return,
            },
            () = fetched.notified() => {}
            () = names.closed() => return,
        }
    }
}

/// 本机已有的已加入成员的显示名，以及本账号的撤回权限。成员读不到时记一条警告，“谁已读”显示账号名，不影响聊天；
/// 权限读不到时按房间的默认权限算（SDK 的 `power_levels_or_default`），服务器才是最后的判断。
pub(crate) async fn local_member_names(room: &Room) -> MemberNames {
    let members = match room.members_no_sync(RoomMemberships::JOIN).await {
        Ok(members) => members,
        Err(error) => {
            tracing::warn!(room = %room.room_id(), "读不到房间成员，已读者显示账号名：{error}");
            Vec::new()
        }
    };
    let power = room.power_levels_or_default().await;
    let own = room.own_user_id();
    let direct = members.len() == 2;
    let avatars = members
        .iter()
        .filter_map(|member| {
            member
                .avatar_url()
                .map(|url| (member.user_id().to_owned(), url.to_string()))
        })
        .collect();
    MemberNames::new(
        members
            .into_iter()
            .map(|member| {
                let name = member.name().to_owned();
                (member.user_id().to_owned(), name)
            })
            .collect::<HashMap<_, _>>(),
        own.to_owned(),
        direct,
    )
    .with_avatars(avatars)
    .with_redaction(RedactionPower {
        own: power.user_can_redact_own_event(own),
        others: power.user_can_redact_event_of_other(own),
    })
}
