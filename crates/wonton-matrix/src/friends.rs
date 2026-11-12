//! 好友（ADR-0051）：好友请求就是带标记的 `is_direct` 私聊邀请，屏蔽用 MSC4155 的 `m.ignored_user_list`。
//!
//! 标记放两处：`m.room.create` 内容里的 [`FRIEND_CREATE_KEY`]，以及同名的房间状态事件。只写状态事件不行：
//! Tuwunel 1.9.4 不给受邀者读房间状态（`GET /rooms/{id}/state` 回 403），Sliding Sync 的 `required_state`
//! 也不给受邀房间带自定义状态，邀请里自带的 stripped state 只有 create、name 这类固定类型。create 内容在
//! stripped state 里，受邀者看得到，入群后也一直在。

use std::collections::BTreeSet;

use matrix_sdk::{
    Room, RoomState,
    ruma::{
        OwnedRoomId, OwnedServerName, OwnedUserId, RoomId, UserId,
        api::{
            client::room::create_room::v3::{Request as CreateRoomRequest, RoomPreset},
            error::ErrorKind,
        },
        events::{StateEventType, room::encryption::RoomEncryptionEventContent},
        serde::Raw,
    },
};
use serde_json::{Value, json};

use crate::{Error, Result};

/// `m.room.create` 内容里的标记键，取值是 `true`。
pub const FRIEND_CREATE_KEY: &str = "app.wonton.friend";
/// MSC4155 的规则列表事件类型（Tuwunel 只认这个不稳定名称）。
const MSC4155_TYPE: &str = "org.matrix.msc4155.invite_permission_config";

/// MSC4155 的拒绝名单：个人屏蔽的用户和整台服务器。空集合表示都不拒绝。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Blocked {
    pub users: BTreeSet<OwnedUserId>,
    pub servers: BTreeSet<OwnedServerName>,
}

/// 撤销的项目从两个名单里一起清掉，别家客户端写的字段保留。
fn strip_ignored(value: &mut Value, users: &[&str], servers: &[&str]) {
    if let Some(list) = value
        .get_mut("ignored_users")
        .and_then(|v| v.as_object_mut())
    {
        list.retain(|id, _| !users.contains(&id.as_str()));
    }
    if let Some(list) = value
        .get_mut("ignored_servers")
        .and_then(|v| v.as_array_mut())
    {
        list.retain(|name| !name.as_str().is_some_and(|name| servers.contains(&name)));
    }
}

/// 收到的一条私聊邀请。`friend` 是带了 [`FRIEND_CREATE_KEY`] 的好友请求，`bot_contact` 是别人家的 Bot 带着
/// [`BOT_CONTACT_CREATE_KEY`](wonton_protocol::BOT_CONTACT_CREATE_KEY) 请他做联系人（ADR-0050）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FriendInvite {
    pub room_id: OwnedRoomId,
    /// 邀请人。
    pub inviter: OwnedUserId,
    pub inviter_name: Option<String>,
    pub invited_at_ms: u64,
    pub friend: bool,
    pub bot_contact: bool,
}

/// 一条已经加入的私聊。`friend` 是建房时带了标记的好友私聊。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FriendChat {
    pub room_id: OwnedRoomId,
    /// 对方；认不出时为空。
    pub peer: Option<OwnedUserId>,
    /// 自己发出的邀请还没被接受时的被邀请人。
    pub pending_invitee: Option<OwnedUserId>,
    pub pending_invitee_name: Option<String>,
    pub invited_at_ms: u64,
    pub friend: bool,
    /// 这个房间是不是本账号建的。
    pub created_by_me: bool,
}

impl super::MatrixSession {
    /// 建一个带好友标记的私聊并邀请 `user_id`，返回房间 ID。加密、私聊标记和普通私聊一样。
    pub async fn invite_friend(&self, user_id: &UserId) -> Result<OwnedRoomId> {
        // 建房时邀请失败，服务器只记日志：对方不存在时房间照样建出来、里面只有自己（Tuwunel 上实测）。
        // 本机用户查得到资料，查不到就是真的不存在；别家服务器的用户资料查不到很常见（联邦的资料查询不一定通），
        // 照样发邀请，由对方服务器回结果。
        let local = user_id.server_name() == self.user_id().server_name();
        if local
            && let Err(error) = self.client.account().fetch_user_profile_of(user_id).await
            && error.client_api_error_kind() == Some(&ErrorKind::NotFound)
        {
            return Err(Error::InvalidInput(format!("找不到用户 {user_id}")));
        }
        let encryption = matrix_sdk::ruma::events::InitialStateEvent::with_empty_state_key(
            RoomEncryptionEventContent::with_recommended_defaults(),
        )
        .to_raw_any();
        let mut request = CreateRoomRequest::new();
        request.creation_content = Some(
            serde_json::from_value::<Raw<_>>(json!({ FRIEND_CREATE_KEY: true }))
                .expect("固定的 JSON 一定能读出 Raw"),
        );
        request.initial_state = vec![encryption];
        request.is_direct = true;
        request.preset = Some(RoomPreset::TrustedPrivateChat);
        let room = self.client.create_room(request).await?;
        // 单独发邀请而不是让建房顺手发：建房里的邀请是尽力而为的，失败了只记服务器日志，客户端看不到原因。
        // 失败时把刚建的房间退掉，不给对面留一个没人进得来的空房间。
        if let Err(error) = room.invite_user_by_id(user_id).await {
            let _ = room.leave().await;
            let _ = room.forget().await;
            return Err(error.into());
        }
        Ok(room.room_id().to_owned())
    }

    /// 收下一条邀请：加入房间，和普通私聊一样记进 `m.direct`。
    pub async fn accept_invite(&self, room_id: &RoomId) -> Result<()> {
        self.join_room(room_id).await
    }

    /// 拒绝一条邀请：退出。房间里只有邀请人，退出就是拒绝。
    pub async fn reject_invite(&self, room_id: &RoomId) -> Result<()> {
        let room = self.room(room_id)?;
        if room.state() != RoomState::Invited {
            return Err(Error::NotReady("这条邀请已经处理过了".to_owned()));
        }
        room.leave().await?;
        room.forget().await?;
        Ok(())
    }

    /// 取消自己发出、对方还没接受的邀请：自己退出并忘掉房间。
    pub async fn cancel_friend_invite(&self, room_id: &RoomId) -> Result<()> {
        let room = self.room(room_id)?;
        room.leave().await?;
        room.forget().await?;
        Ok(())
    }

    /// 一条还没处理的私聊邀请；不是邀请、已经处理过或邀请人已经取消时为空。
    pub async fn friend_invite(&self, room_id: &RoomId) -> Result<Option<FriendInvite>> {
        let Ok(room) = self.room(room_id) else {
            return Ok(None);
        };
        if room.state() != RoomState::Invited || !(is_direct(&room).await || is_friend(&room).await)
        {
            return Ok(None);
        }
        let details = room.invite_details().await?;
        // 邀请人取消就是自己退出了；邀请还留在本机，但已经不是待处理的请求。
        if let Ok(Some(inviter)) = room.get_member_no_sync(&details.inviter_id).await
            && inviter.membership()
                == &matrix_sdk::ruma::events::room::member::MembershipState::Leave
        {
            return Ok(None);
        }
        // 邀请里的 stripped 成员事件常常没有显示名，资料去服务器要；要不到就用用户 ID。
        let inviter_name = match self
            .client
            .account()
            .fetch_user_profile_of(&details.inviter_id)
            .await
        {
            Ok(profile) => profile
                .data
                .get("displayname")
                .and_then(|value| value.as_str())
                .map(str::to_owned),
            Err(error) => {
                tracing::warn!(inviter = %details.inviter_id, "取邀请人的资料失败：{error}");
                None
            }
        };
        let invited_at_ms = created_at_ms(&room).await;
        Ok(Some(FriendInvite {
            room_id: room.room_id().to_owned(),
            inviter_name,
            inviter: details.inviter_id,
            invited_at_ms,
            friend: is_friend(&room).await,
            bot_contact: created_with(&room, wonton_protocol::BOT_CONTACT_CREATE_KEY).await,
        }))
    }

    /// 一条已加入的私聊；不是私聊时为空。自己发出、对方还没接受的邀请里 `pending_invitee` 是对方。
    pub async fn friend_chat(&self, room_id: &RoomId) -> Result<Option<FriendChat>> {
        let Ok(room) = self.room(room_id) else {
            return Ok(None);
        };
        if room.state() != RoomState::Joined || !(is_direct(&room).await || is_friend(&room).await)
        {
            return Ok(None);
        }
        let peer = room
            .direct_targets()
            .into_iter()
            .find_map(|target| target.as_user_id().map(ToOwned::to_owned));
        let peer = match peer {
            Some(peer) => Some(peer),
            None => room
                .members(matrix_sdk::RoomMemberships::JOIN)
                .await?
                .into_iter()
                .find(|member| member.user_id() != room.own_user_id())
                .map(|member| member.user_id().to_owned()),
        };
        let mut pending = room
            .members_no_sync(matrix_sdk::RoomMemberships::INVITE)
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|member| member.user_id() != room.own_user_id());
        let first = pending.next();
        let pending_invitee = first.as_ref().map(|member| member.user_id().to_owned());
        let pending_invitee_name = first
            .as_ref()
            .and_then(|member| member.display_name().map(str::to_owned));
        let invited_at_ms = created_at_ms(&room).await;
        let created_by_me = room
            .get_state_event(StateEventType::RoomCreate, "")
            .await
            .ok()
            .flatten()
            .and_then(|raw| serde_json::to_value(raw).ok())
            .and_then(|value| value["sender"].as_str().map(str::to_owned))
            .is_some_and(|sender| sender == room.own_user_id().as_str());
        Ok(Some(FriendChat {
            room_id: room.room_id().to_owned(),
            peer,
            pending_invitee,
            pending_invitee_name,
            invited_at_ms,
            friend: is_friend(&room).await,
            created_by_me,
        }))
    }

    /// 当前的 MSC4155 拒绝名单，合并 `m.ignored_user_list` 的忽略项。没写过时是空的。
    pub async fn blocked_list(&self) -> Result<Blocked> {
        // 读服务器上最新的：本机缓存要等下一次同步才更新，刚写完就读会读到旧值。
        let msc = self.fetch_json(MSC4155_TYPE).await?;
        let ignored = self.fetch_json("m.ignored_user_list").await?;
        let mut blocked = Blocked::default();
        for value in [&msc, &ignored] {
            if let Some(users) = value.get("blocked_users").and_then(|v| v.as_array()) {
                for id in users {
                    if let Some(id) = id.as_str().and_then(|id| id.parse().ok()) {
                        blocked.users.insert(id);
                    }
                }
            }
            if let Some(servers) = value.get("blocked_servers").and_then(|v| v.as_array()) {
                for name in servers {
                    if let Some(name) = name.as_str().and_then(|name| name.parse().ok()) {
                        blocked.servers.insert(name);
                    }
                }
            }
        }
        if let Some(users) = ignored.get("ignored_users").and_then(|v| v.as_object()) {
            for id in users.keys() {
                if let Ok(id) = id.parse() {
                    blocked.users.insert(id);
                }
            }
        }
        if let Some(servers) = ignored.get("ignored_servers").and_then(|v| v.as_array()) {
            for name in servers {
                if let Some(name) = name.as_str().and_then(|name| name.parse().ok()) {
                    blocked.servers.insert(name);
                }
            }
        }
        Ok(blocked)
    }

    /// 整份写回拒绝名单。别家客户端写的字段（`enabled`、`allowed_*`）原样保留；撤销的项也从忽略名单里清掉。
    pub async fn set_blocked_list(&self, blocked: &Blocked) -> Result<()> {
        let users: Vec<&str> = blocked.users.iter().map(|id| id.as_str()).collect();
        let servers: Vec<&str> = blocked.servers.iter().map(|name| name.host()).collect();
        let mut msc = self.fetch_json(MSC4155_TYPE).await?;
        if !msc.is_object() {
            msc = json!({});
        }
        strip_ignored(&mut msc, &users, &servers);
        msc["blocked_users"] = json!(users);
        msc["blocked_servers"] = json!(servers);
        self.set_json(MSC4155_TYPE, &msc).await?;

        let mut ignored = self.fetch_json("m.ignored_user_list").await?;
        if !ignored.is_object() {
            ignored = json!({});
        }
        strip_ignored(&mut ignored, &users, &servers);
        self.set_json("m.ignored_user_list", &ignored).await
    }

    /// 取一个账号数据事件的内容；没写过时是 `null`。
    async fn fetch_json(&self, event_type: &str) -> Result<serde_json::Value> {
        let raw = self
            .client
            .account()
            .fetch_account_data(matrix_sdk::ruma::events::GlobalAccountDataEventType::from(
                event_type,
            ))
            .await?;
        match raw {
            Some(raw) => {
                serde_json::to_value(&raw).map_err(|error| Error::Upstream(error.to_string()))
            }
            None => Ok(serde_json::Value::Null),
        }
    }

    /// 写一个账号数据事件的内容。
    async fn set_json(&self, event_type: &str, value: &serde_json::Value) -> Result<()> {
        let raw = serde_json::from_value::<Raw<_>>(value.clone())
            .map_err(|error| Error::Upstream(error.to_string()))?;
        self.client
            .account()
            .set_account_data_raw(
                matrix_sdk::ruma::events::GlobalAccountDataEventType::from(event_type),
                raw,
            )
            .await?;
        Ok(())
    }
}

/// 房间建的时刻，Unix 毫秒；邀请里的其他事件常常没有时间戳，拿建房时间当邀请时间。读不到时为零。
async fn created_at_ms(room: &Room) -> u64 {
    room.get_state_event(StateEventType::RoomCreate, "")
        .await
        .ok()
        .flatten()
        .and_then(|raw| serde_json::to_value(raw).ok())
        .and_then(|value| value["origin_server_ts"].as_u64())
        .unwrap_or_default()
}

/// 房间建的时候带没带好友标记。
///
/// 只认 `m.room.create` 里那个不可改的标记：同名的状态事件谁都能后加，一条普通私聊补上它就会突然变成好友，
/// 删掉的好友也会因此复活。代价是外部客户端建的房间认不出来，第一个外部用户之前不需要兼容。
async fn is_friend(room: &Room) -> bool {
    created_with(room, FRIEND_CREATE_KEY).await
}

/// 建房时 `m.room.create` 内容里的这个标记是不是 `true`。
async fn created_with(room: &Room, key: &str) -> bool {
    room.get_state_event(StateEventType::RoomCreate, "")
        .await
        .ok()
        .flatten()
        .and_then(|raw| serde_json::to_value(raw).ok())
        .is_some_and(|value| value["content"][key].as_bool() == Some(true))
}

async fn is_direct(room: &Room) -> bool {
    room.is_direct().await.unwrap_or_else(|error| {
        tracing::warn!(room = %room.room_id(), "读不到私聊标记，按群聊算：{error}");
        false
    })
}

/// `room_info` 用：房间带没带好友标记。
pub(crate) async fn marked_friend(room: &Room) -> bool {
    is_friend(room).await
}
