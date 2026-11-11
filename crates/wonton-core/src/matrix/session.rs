//! 以某个用户的身份调 Tuwunel 的客户端接口：核心代 Bot 设置资料、建主房间、邀请主人，代主人加入它。

use hyper::Method;
use serde_json::{Map, Value, json};

use super::api::{Api, MatrixError, encode};
use crate::l10n;
use crate::plugin::languages::Languages;

/// 一个已登录用户的会话。令牌属于核心刚登录的设备，用完由调用方吊销或退出。
pub struct Session<'a> {
    pub(super) api: &'a Api,
    user_id: String,
    pub(super) token: String,
}

impl<'a> Session<'a> {
    pub fn new(api: &'a Api, user_id: &str, token: &str) -> Self {
        Self {
            api,
            user_id: user_id.to_owned(),
            token: token.to_owned(),
        }
    }

    pub fn user_id(&self) -> &str {
        &self.user_id
    }

    pub(super) async fn json(
        &self,
        method: Method,
        path: &str,
        body: Option<&Value>,
        context: &str,
    ) -> Result<Value, MatrixError> {
        self.api
            .json(method, path, Some(&self.token), body, context)
            .await
    }

    pub(super) fn profile_path(&self, field: &str) -> String {
        format!(
            "/_matrix/client/v3/profile/{}/{field}",
            encode(&self.user_id)
        )
    }

    /// 一个全局账号数据事件的内容；没设过时是 `None`。
    pub async fn account_data(&self, event: &str) -> Result<Option<Value>, MatrixError> {
        let path = format!(
            "/_matrix/client/v3/user/{}/account_data/{}",
            encode(&self.user_id),
            encode(event)
        );
        match self.json(Method::GET, &path, None, "读账号数据").await {
            Ok(value) => Ok(Some(value)),
            Err(e) if e.errcode() == "M_NOT_FOUND" => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// 一个房间账号数据事件的内容；没设过时是 `None`。
    pub async fn room_account_data(
        &self,
        room: &str,
        event: &str,
    ) -> Result<Option<Value>, MatrixError> {
        let path = format!(
            "/_matrix/client/v3/user/{}/rooms/{}/account_data/{}",
            encode(&self.user_id),
            encode(room),
            encode(event)
        );
        match self.json(Method::GET, &path, None, "读房间账号数据").await {
            Ok(value) => Ok(Some(value)),
            Err(e) if e.errcode() == "M_NOT_FOUND" => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// 账号的显示名。Matrix 规定没设时回 `M_NOT_FOUND`。
    pub async fn display_name(&self) -> Result<Option<String>, MatrixError> {
        let path = self.profile_path("displayname");
        match self.json(Method::GET, &path, None, "读显示名").await {
            Ok(profile) => super::bot_profile::profile_string(&profile, "displayname"),
            Err(e) if e.errcode() == "M_NOT_FOUND" => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub async fn set_display_name(&self, name: &str) -> Result<(), MatrixError> {
        let body = json!({"displayname": name});
        self.json(
            Method::PUT,
            &self.profile_path("displayname"),
            Some(&body),
            "设置显示名",
        )
        .await
        .map(|_| ())
    }

    /// 上传头像并设为自己的头像。
    pub async fn set_avatar(
        &self,
        content_type: &str,
        filename: &str,
        data: Vec<u8>,
    ) -> Result<(), MatrixError> {
        let uri = self.upload_media(content_type, filename, data).await?;
        self.set_avatar_url(Some(&uri)).await
    }

    /// 建一个给自己和 `peer` 用的加密房间，返回房间 ID。先不邀请对方：Bot 装好了才交给主人（[`Self::invite`]）。
    ///
    /// 房间版本 12 里建房的人权限最高，不能写进 power_levels 的 `users`（Tuwunel 回 400）。`trusted_private_chat`
    /// 把建房时邀请的人也算作建房的人，之后邀请的不算，所以在 `users` 里给对方 100：对方要是建房的人，自己就踢不动
    /// 对方、收不回私聊。版本写死：版本 11 里 `users` 会被整个换掉，建房的人只剩 0，建房失败（Tuwunel v1.9.4 实测）。
    pub async fn create_direct_room(&self, peer: &str) -> Result<String, MatrixError> {
        let body = json!({
            "preset": "trusted_private_chat",
            "room_version": "12",
            "power_level_content_override": {"users": {peer: 100}},
            "initial_state": [{
                "type": "m.room.encryption",
                "state_key": "",
                "content": {"algorithm": "m.megolm.v1.aes-sha2"},
            }],
        });
        let value = self
            .json(
                Method::POST,
                "/_matrix/client/v3/createRoom",
                Some(&body),
                "建私聊",
            )
            .await?;
        value["room_id"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| MatrixError::Invalid("建房接口没有返回 room_id".to_owned()))
    }

    /// 建一个和 `invitee` 的不加密私聊（核心发给主人的通知房间，ADR-0016）并邀请对方，返回房间 ID。
    pub async fn create_notice_room(
        &self,
        name: &str,
        invitee: &str,
    ) -> Result<String, MatrixError> {
        let body = json!({
            "preset": "trusted_private_chat",
            "is_direct": true,
            "name": name,
            "invite": [invitee],
        });
        let value = self
            .json(
                Method::POST,
                "/_matrix/client/v3/createRoom",
                Some(&body),
                "建通知房间",
            )
            .await?;
        value["room_id"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| MatrixError::Invalid("建房接口没有返回 room_id".to_owned()))
    }

    /// 发一条消息，返回事件 ID。
    pub async fn send_message(&self, room: &str, content: &Value) -> Result<String, MatrixError> {
        self.send_message_with_txn(room, content, &crate::secret::random_token())
            .await
    }

    /// 同一事务可在断线或重启后重发，Matrix 不会在同一房间里生成重复事件。
    pub(crate) async fn send_message_with_txn(
        &self,
        room: &str,
        content: &Value,
        transaction: &str,
    ) -> Result<String, MatrixError> {
        let path = format!(
            "/_matrix/client/v3/rooms/{}/send/m.room.message/{}",
            encode(room),
            encode(transaction)
        );
        let value = self
            .json(Method::PUT, &path, Some(content), "发消息")
            .await?;
        value["event_id"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| MatrixError::Invalid("发消息接口没有返回 event_id".to_owned()))
    }

    pub async fn join(&self, room: &str) -> Result<(), MatrixError> {
        let path = format!("/_matrix/client/v3/join/{}", encode(room));
        self.json(Method::POST, &path, Some(&json!({})), "加入房间")
            .await
            .map(|_| ())
    }

    pub async fn invite(&self, room: &str, user: &str) -> Result<(), MatrixError> {
        let path = format!("/_matrix/client/v3/rooms/{}/invite", encode(room));
        let body = json!({"user_id": user});
        self.json(Method::POST, &path, Some(&body), "邀请")
            .await
            .map(|_| ())
    }

    /// `user` 在房间里的成员状态（`join`、`invite`、`leave`、`ban`）；从来没进过这个房间时是 `None`。
    pub async fn membership(&self, room: &str, user: &str) -> Result<Option<String>, MatrixError> {
        let path = format!(
            "/_matrix/client/v3/rooms/{}/state/m.room.member/{}",
            encode(room),
            encode(user)
        );
        match self.json(Method::GET, &path, None, "查询成员状态").await {
            Ok(value) => Ok(value["membership"].as_str().map(str::to_owned)),
            Err(e) if e.errcode() == "M_NOT_FOUND" => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// 撤回对 `user` 的邀请并退出房间：建好的私聊没能交到对方手里时用，免得对方收到一个没人应答的邀请。
    pub async fn abandon(
        &self,
        room: &str,
        user: &str,
        langs: &Languages,
    ) -> Result<(), MatrixError> {
        let room = encode(room);
        let reason = l10n::text(langs, "chat-kick-reason", &[]);
        let kick = json!({"user_id": user, "reason": reason});
        let path = format!("/_matrix/client/v3/rooms/{room}/kick");
        self.json(Method::POST, &path, Some(&kick), "撤回邀请")
            .await?;
        let path = format!("/_matrix/client/v3/rooms/{room}/leave");
        self.json(Method::POST, &path, Some(&json!({})), "退出房间")
            .await
            .map(|_| ())
    }

    /// 房间里已加入的成员。
    pub async fn joined_members(&self, room: &str) -> Result<Vec<String>, MatrixError> {
        let path = format!("/_matrix/client/v3/rooms/{}/joined_members", encode(room));
        let value = self.json(Method::GET, &path, None, "查询房间成员").await?;
        Ok(value["joined"]
            .as_object()
            .map(|joined| joined.keys().cloned().collect())
            .unwrap_or_default())
    }

    /// 在自己的 `m.direct` 里把 `room` 记为和 `other` 的私聊（保留已有的记录）。
    pub async fn add_direct(&self, other: &str, room: &str) -> Result<(), MatrixError> {
        let path = format!(
            "/_matrix/client/v3/user/{}/account_data/m.direct",
            encode(&self.user_id)
        );
        let mut direct = match self.json(Method::GET, &path, None, "读取 m.direct").await {
            Ok(Value::Object(map)) => map,
            Ok(_) => Map::new(),
            Err(e) if e.errcode() == "M_NOT_FOUND" => Map::new(),
            Err(e) => return Err(e),
        };
        let rooms = direct.entry(other.to_owned()).or_insert_with(|| json!([]));
        match rooms.as_array_mut() {
            Some(list) if list.iter().any(|r| r == room) => {}
            Some(list) => list.push(json!(room)),
            None => *rooms = json!([room]),
        }
        self.json(
            Method::PUT,
            &path,
            Some(&Value::Object(direct)),
            "写入 m.direct",
        )
        .await
        .map(|_| ())
    }

    /// 不再用这个和 `other` 的私聊：退出房间，从 `m.direct` 去掉。收拾不掉只记日志，新的房间已经能用了。
    pub async fn retire_direct(&self, other: &str, room: &str) {
        let path = format!("/_matrix/client/v3/rooms/{}/leave", encode(room));
        let left = self
            .json(Method::POST, &path, Some(&json!({})), "退出房间")
            .await;
        match left {
            Err(e) if e.errcode() != "M_FORBIDDEN" && e.errcode() != "M_NOT_FOUND" => {
                tracing::warn!(room, error = %e, "退出旧的私聊失败");
            }
            _ => {}
        }
        if let Err(e) = self.remove_direct(other, room).await {
            tracing::warn!(room, error = %e, "从 m.direct 去掉旧的私聊失败");
        }
    }

    async fn remove_direct(&self, other: &str, room: &str) -> Result<(), MatrixError> {
        let path = format!(
            "/_matrix/client/v3/user/{}/account_data/m.direct",
            encode(&self.user_id)
        );
        let mut direct = match self.json(Method::GET, &path, None, "读取 m.direct").await {
            Ok(Value::Object(map)) => map,
            Err(e) if e.errcode() != "M_NOT_FOUND" => return Err(e),
            _ => return Ok(()),
        };
        let Some(rooms) = direct.get_mut(other).and_then(Value::as_array_mut) else {
            return Ok(());
        };
        rooms.retain(|r| r != room);
        self.json(
            Method::PUT,
            &path,
            Some(&Value::Object(direct)),
            "写入 m.direct",
        )
        .await
        .map(|_| ())
    }

    /// 退出登录：这台设备和它的令牌一起作废。
    pub async fn logout(&self) -> Result<(), MatrixError> {
        self.json(
            Method::POST,
            "/_matrix/client/v3/logout",
            Some(&json!({})),
            "退出登录",
        )
        .await
        .map(|_| ())
    }
}
