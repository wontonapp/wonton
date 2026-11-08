//! 房间主人：`m.room.create` 的发送者是本服务器上的人、又不是这个 Bot 时就是他，
//! 否则是 Bot 的主人。取不到创建事件按 Bot 的主人算。

use wonton_matrix::{OwnedUserId, UserId};

use crate::room::Room;

/// 房间主人：`m.room.create` 的发送者是本服务器上的人、又不是这个 Bot 时就是他，
/// 否则是 Bot 的主人。取不到创建事件或发送者按 Bot 的主人算。
pub(crate) async fn room_owner(room: &impl Room, bot: &UserId, bot_owner: &UserId) -> OwnedUserId {
    let created = match room.created().await {
        Ok(created) => created,
        Err(error) => {
            tracing::warn!(error = %error, "can't read the room's create event; treating the bot owner as the room owner");
            return bot_owner.to_owned();
        }
    };
    let sender = created.as_ref().and_then(|event| event["sender"].as_str());
    if sender.is_some_and(|sender| sender.parse::<OwnedUserId>().is_err()) {
        tracing::warn!(
            "the room's create event has no usable sender; treating the bot owner as the room owner"
        );
    }
    room_owner_id(created.as_ref(), bot, bot_owner)
}

/// `m.room.create` 的发送者是本服务器上的人、又不是这个 Bot 时用他，否则用 Bot 的主人。
pub(crate) fn room_owner_id(
    created: Option<&serde_json::Value>,
    bot: &UserId,
    bot_owner: &UserId,
) -> OwnedUserId {
    let creator = created
        .and_then(|event| event["sender"].as_str())
        .and_then(|sender| sender.parse::<OwnedUserId>().ok());
    match creator {
        Some(creator) if creator != *bot && creator.server_name() == bot.server_name() => creator,
        Some(_) => bot_owner.to_owned(),
        None => bot_owner.to_owned(),
    }
}
