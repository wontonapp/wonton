//! Siri、快捷指令的“问 Bot”：一句话直接发进和默认 Agent 的私聊（ADR-0041）。

use std::sync::Arc;

use wonton_l10n::l10n;
use wonton_matrix::{OwnedRoomId, OwnedUserId};
use wonton_protocol::{CoreError, RoomMembership};

use super::{ChatLink, invalid, onboarding};
use crate::l10n::{CATALOG, language};

impl ChatLink {
    /// 发给 Bot 的一句话（Siri、快捷指令的“问 Bot”）：和它的私聊还没有就先建，建好再发，和发起私聊一样加密。
    pub(crate) fn ask(self: &Arc<Self>, bot: &str, text: &str) -> Result<(), CoreError> {
        let body = text.trim().to_owned();
        if body.is_empty() {
            return Err(invalid(&l10n!(CATALOG, language(), "chat-empty-message")));
        }
        let bot: OwnedUserId = bot
            .parse()
            .map_err(|_| invalid(&l10n!(CATALOG, language(), "chat-ask-invalid-bot-id")))?;
        let session = self.current()?;
        let sent = self.sent.clone();
        // 按列表找和 Bot 的私聊：Bot 先发起、主人还没接受的邀请也算（SDK 的私聊表要加入后才记），已加入的优先。
        let existing = {
            let view = self.view.borrow();
            let rooms = || {
                view.rooms
                    .iter()
                    .filter(|row| row.direct_user_id.as_deref() == Some(bot.as_str()))
            };
            rooms()
                .find(|row| row.membership == RoomMembership::Joined)
                .or_else(|| rooms().find(|row| row.membership == RoomMembership::Invited))
                .map(|row| (row.room_id.clone(), row.membership))
        };
        let existing = match existing {
            Some((id, membership)) => Some((
                OwnedRoomId::try_from(id).map_err(|_| {
                    invalid(&l10n!(CATALOG, language(), "chat-ask-invalid-room-id"))
                })?,
                membership,
            )),
            None => None,
        };
        self.spawn_op("chat-op-ask-agent-failed", async move {
            let room_id = match existing {
                Some((room_id, RoomMembership::Invited)) => {
                    session.join_room(&room_id).await?;
                    room_id
                }
                Some((room_id, _)) => room_id,
                None => session.create_direct_chat(&bot, true).await?,
            };
            session.send_text_to(&room_id, &body).await?;
            onboarding::record_sent(&sent, room_id.to_string());
            Ok(())
        });
        Ok(())
    }
}
