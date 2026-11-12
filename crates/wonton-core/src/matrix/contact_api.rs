//! 联系人（ADR-0045、ADR-0050）用到的客户端接口：两台核心之间的联系人房间，和 Bot 给别人家的人发的带标记私聊邀请。

use hyper::Method;
use serde_json::{Value, json};
use wonton_protocol::{BOT_CONTACT_CREATE_KEY, CONTACT_ROOM_TYPE};

use super::api::{MatrixError, encode};
use super::session::Session;

impl Session<'_> {
    /// 建两台核心之间的联系人房间（[`CONTACT_ROOM_TYPE`]）并邀请对方的核心账号，返回房间 ID。不加密：核心没有加密设备，
    /// 内容是两台核心之间的请求和结果，不含聊天。邀请单独发，失败时房间已经建好，由调用方记下、稍后重发邀请。
    pub async fn create_contact_room(&self) -> Result<String, MatrixError> {
        let body = json!({
            "preset": "private_chat",
            "creation_content": {"type": CONTACT_ROOM_TYPE},
        });
        let value = self
            .json(
                Method::POST,
                "/_matrix/client/v3/createRoom",
                Some(&body),
                "建联系人房间",
            )
            .await?;
        value["room_id"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| MatrixError::Invalid("建房接口没有返回 room_id".to_owned()))
    }

    /// 经 `server` 加入别的服务器上的房间：跨机加入要告诉 Tuwunel 去哪台服务器握手。
    pub async fn join_via(&self, room: &str, server: &str) -> Result<(), MatrixError> {
        let path = format!(
            "/_matrix/client/v3/join/{}?server_name={}",
            encode(room),
            encode(server)
        );
        self.json(Method::POST, &path, Some(&json!({})), "加入房间")
            .await
            .map(|_| ())
    }

    /// 发一条自定义类型的消息事件。同一 `transaction` 重发不会生成重复事件。
    pub async fn send_event(
        &self,
        room: &str,
        kind: &str,
        content: &Value,
        transaction: &str,
    ) -> Result<String, MatrixError> {
        let path = format!(
            "/_matrix/client/v3/rooms/{}/send/{}/{}",
            encode(room),
            encode(kind),
            encode(transaction)
        );
        let value = self
            .json(Method::PUT, &path, Some(content), "发事件")
            .await?;
        value["event_id"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| MatrixError::Invalid("发事件接口没有返回 event_id".to_owned()))
    }

    /// 一次 `/sync`。`filter` 是内联的过滤器 JSON；`timeout_ms` 要比接口超时（30 秒）短。
    pub async fn sync(
        &self,
        since: Option<&str>,
        filter: &Value,
        timeout_ms: u64,
    ) -> Result<Value, MatrixError> {
        let mut path = format!(
            "/_matrix/client/v3/sync?timeout={timeout_ms}&filter={}",
            encode(&filter.to_string())
        );
        if let Some(since) = since {
            path.push_str("&since=");
            path.push_str(&encode(since));
        }
        self.json(Method::GET, &path, None, "同步").await
    }

    /// 建一个带 [`BOT_CONTACT_CREATE_KEY`] 的加密私聊并邀请 `user`，返回房间 ID。邀请单独发：建房时顺带的邀请失败了
    /// 只记服务器日志（同 wonton-matrix 的好友邀请）。邀请失败时退出刚建的房间，把邀请的错误交给调用方分类。
    pub async fn invite_as_contact(&self, user: &str) -> Result<String, MatrixError> {
        let body = json!({
            "preset": "trusted_private_chat",
            "is_direct": true,
            "creation_content": {BOT_CONTACT_CREATE_KEY: true},
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
                "建联系人私聊",
            )
            .await?;
        let room = value["room_id"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| MatrixError::Invalid("建房接口没有返回 room_id".to_owned()))?;
        if let Err(error) = self.invite(&room, user).await {
            self.leave(&room).await;
            return Err(error);
        }
        if let Err(error) = self.add_direct(user, &room).await {
            tracing::warn!(%room, error = %error, "联系人私聊没记进 m.direct");
        }
        Ok(room)
    }

    /// 撤回对 `user` 的邀请（踢出），不退出房间：对方已经接受时什么都不做。
    pub async fn withdraw_invite(&self, room: &str, user: &str) -> Result<(), MatrixError> {
        if self.membership(room, user).await?.as_deref() != Some("invite") {
            return Ok(());
        }
        let path = format!("/_matrix/client/v3/rooms/{}/kick", encode(room));
        let body = json!({"user_id": user});
        self.json(Method::POST, &path, Some(&body), "撤回邀请")
            .await
            .map(|_| ())
    }

    /// 尽力退出房间：只在清理半成品时用，失败只记日志。
    async fn leave(&self, room: &str) {
        let path = format!("/_matrix/client/v3/rooms/{}/leave", encode(room));
        if let Err(error) = self
            .json(Method::POST, &path, Some(&json!({})), "退出房间")
            .await
        {
            tracing::warn!(%room, error = %error, "退出没邀请成的房间失败");
        }
    }
}
