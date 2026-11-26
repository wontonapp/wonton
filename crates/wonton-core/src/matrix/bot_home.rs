//! 主房间由主人建、由主人邀请 Bot 进来：插件适配器按“邀请人在允许名单里”收邀请，Bot 自己不在名单里，
//! 由 Bot 建（进房的事件发送人是它自己）就会被适配器当成陌生人丢掉。主人当房主，撤回时 Bot 踢不动他
//! （两个建房人之间也踢不动，Tuwunel 回 403），所以把 Bot 也写进 `additional_creators`，撤回时由主人自己退。
//! Bot 接受邀请后主人马上退出：房间从主人的聊天列表里消失，交付（`hand_over`）时再请回来。
//!
//! 建房时写的一个标记让重试能捡回上次建了一半的主房间。能捡回的是 Bot 已经进来的那一段（建房、邀请、Bot
//! 加入都做完了，后面的设置或交付崩了）。邀请或 Bot 加入就失败时 Bot 还没进来，捡不回来；`createRoom` 的响应
//! 丢掉时连 room_id 都没有。这两种都让主人退出（响应丢掉时按建房号把房间认出来），房间留在服务器上、主人看不到，
//! 重试另建一间。

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU64, Ordering};

use hyper::Method;
use serde_json::{Value, json};

use super::Session;
use super::api::{MatrixError, encode};
use crate::plugin::languages::Languages;

const MARKER: &str = "app.wonton.bot_home";

/// 每次建房用一个号写进标记：并发装两个插件时，失败的那一路只退出自己那间，不动另一路刚建的房。
static NEXT_RESERVATION: AtomicU64 = AtomicU64::new(1);

/// 取一个建房号；进程内唯一。重启后从 1 再数，早先的房间在快照 `before` 里，撞号也不影响。
pub(super) fn next_reservation() -> u64 {
    NEXT_RESERVATION.fetch_add(1, Ordering::Relaxed)
}

impl Session<'_> {
    /// 捡回上次建了一半的主房间：有标记、主人对得上、房间还能用。标记的发送人是建房的人（主人），
    /// 旧版本留下的房间标记是 Bot 自己，两种都认。`handed_over` 同 [`reusable`](super::bot::reusable)：主人退出过的
    /// 房间只有没交出去过才捡。
    pub(super) async fn recover_bot_home(
        &self,
        owner: &str,
        handed_over: bool,
    ) -> Result<Option<String>, MatrixError> {
        let rooms = self.joined_rooms().await?;
        let mut found = None;
        for room in &rooms {
            let path = format!("/_matrix/client/v3/rooms/{}/state", encode(room));
            let state = self
                .json(Method::GET, &path, None, "inspect bot home marker")
                .await?;
            if marked_as_bot_home(&state, self.user_id(), owner, None)
                && super::bot::reusable(self, room, owner, handed_over).await?
            {
                if found.is_some() {
                    return Err(MatrixError::Invalid(
                        "multiple bot home reservations".into(),
                    ));
                }
                found = Some(room.clone());
            }
        }
        Ok(found)
    }

    /// 主人现在加入的房间。
    pub(super) async fn joined_rooms(&self) -> Result<BTreeSet<String>, MatrixError> {
        let rooms = self
            .json(
                Method::GET,
                "/_matrix/client/v3/joined_rooms",
                None,
                "list joined rooms",
            )
            .await?;
        Ok(rooms["joined_rooms"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|room| room.as_str().map(str::to_owned))
            .collect())
    }

    /// `createRoom` 的响应丢掉时房间可能已经建好、主人正留在里面，而客户端没拿到 room_id。[`Self::joined_rooms`]
    /// 的快照 `before` 之外、带这次建房号（`reservation`）标记的房间就是这次新建的：让主人退出后再报原错误。
    pub(super) async fn leave_new_bot_homes(
        &self,
        owner: &str,
        before: &BTreeSet<String>,
        reservation: u64,
    ) -> Result<(), MatrixError> {
        for room in self.joined_rooms().await?.difference(before) {
            let path = format!("/_matrix/client/v3/rooms/{}/state", encode(room));
            let state = self
                .json(Method::GET, &path, None, "inspect new bot home")
                .await?;
            if marked_as_bot_home(&state, self.user_id(), owner, Some(reservation)) {
                self.leave_room(room, &Languages::default()).await?;
            }
        }
        Ok(())
    }

    /// 建房：带标记，把 `bot` 写进 `additional_creators`，让它和建房人一样是 creator 级（主人是建房人，
    /// Bot 踢不动他，靠提高自己权限没有用）。建房的人不能写进 `power_level_content_override.users`
    /// （`M_INVALID_PARAM` 400），所以两人都不写，都用 creator 的默认 100。用主人的会话调。邀请、加入由调用方接着做。
    pub(super) async fn create_bot_home(
        &self,
        owner: &str,
        bot: &str,
        reservation: u64,
    ) -> Result<String, MatrixError> {
        let body = json!({
            "preset": "trusted_private_chat",
            "room_version": "12",
            "creation_content": {"additional_creators": [bot]},
            "initial_state": [
                {"type": MARKER, "state_key": "", "content": {"owner": owner, "reservation": reservation}},
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

/// 房间状态里有没有属于 `owner` 的主房间标记；标记的发送人是建房的人（主人，旧版本是 Bot）。给了建房号时还要对上号。
fn marked_as_bot_home(state: &Value, user: &str, owner: &str, reservation: Option<u64>) -> bool {
    state.as_array().into_iter().flatten().any(|event| {
        event["type"] == MARKER
            && event["state_key"] == ""
            && (event["sender"] == user || event["sender"] == owner)
            && event["content"]["owner"] == owner
            && reservation.is_none_or(|reservation| {
                event["content"]["reservation"].as_u64() == Some(reservation)
            })
    })
}
