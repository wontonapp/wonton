//! 主房间由主人建、由主人邀请 Bot 进来：插件适配器按“邀请人在允许名单里”收邀请，Bot 自己不在名单里，
//! 由 Bot 建（进房的事件发送人是它自己）就会被适配器当成陌生人丢掉。主人当房主，撤回时 Bot 踢不动他
//! （两个建房人之间也踢不动，Tuwunel 回 403），所以把 Bot 也写进 `additional_creators`，撤回时由主人自己退。
//!
//! 建房时写的一个标记让重试能捡回响应丢掉的那次建房。

use hyper::Method;
use serde_json::json;

use super::Session;
use super::api::{MatrixError, encode};

const MARKER: &str = "app.wonton.bot_home";

impl Session<'_> {
    /// 捡回上次建了一半的主房间：有标记、主人对得上、房间还能用。标记的发送人是建房的人（主人），
    /// 旧版本留下的房间标记是 Bot 自己，两种都认。
    pub(super) async fn recover_bot_home(
        &self,
        owner: &str,
    ) -> Result<Option<String>, MatrixError> {
        let rooms = self
            .json(
                Method::GET,
                "/_matrix/client/v3/joined_rooms",
                None,
                "recover bot home",
            )
            .await?;
        let mut found = None;
        for room in rooms["joined_rooms"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|room| room.as_str())
        {
            let path = format!("/_matrix/client/v3/rooms/{}/state", encode(room));
            let state = self
                .json(Method::GET, &path, None, "inspect bot home marker")
                .await?;
            let marked = state.as_array().into_iter().flatten().any(|event| {
                event["type"] == MARKER
                    && event["state_key"] == ""
                    && (event["sender"] == self.user_id() || event["sender"] == owner)
                    && event["content"]["owner"] == owner
            });
            if marked && super::bot::reusable(self, room, owner).await? {
                if found.is_some() {
                    return Err(MatrixError::Invalid(
                        "multiple bot home reservations".into(),
                    ));
                }
                found = Some(room.to_owned());
            }
        }
        Ok(found)
    }

    /// 建房：带标记和加密，把 `bot` 写进 `additional_creators`，让它和建房人一样是 creator 级（主人是建房人，
    /// Bot 踢不动他，靠提高自己权限没有用）。建房的人不能写进 `power_level_content_override.users`
    /// （`M_INVALID_PARAM` 400），所以两人都不写，都用 creator 的默认 100。用主人的会话调。邀请、加入由调用方接着做。
    pub(super) async fn create_bot_home(
        &self,
        owner: &str,
        bot: &str,
    ) -> Result<String, MatrixError> {
        let body = json!({
            "preset": "trusted_private_chat",
            "room_version": "12",
            "creation_content": {"additional_creators": [bot]},
            "initial_state": [
                {"type": "m.room.encryption", "state_key": "", "content": {"algorithm": "m.megolm.v1.aes-sha2"}},
                {"type": MARKER, "state_key": "", "content": {"owner": owner}},
            ],
        });
        let room = self
            .json(
                Method::POST,
                "/_matrix/client/v3/createRoom",
                Some(&body),
                "create bot home",
            )
            .await?;
        room["room_id"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| MatrixError::Invalid("createRoom returned no room_id".into()))
    }
}
