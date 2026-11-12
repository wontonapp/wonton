//! Matrix 链接的房间目标：别名交给 SDK 解出真实 ID，导航不改变房间成员关系。

use wonton_l10n::l10n;
use wonton_protocol::CoreError;

use super::{ChatLink, view::core_error};
use crate::l10n::{CATALOG, language};

impl ChatLink {
    pub(crate) async fn resolve_chat_room(&self, input: &str) -> Result<String, CoreError> {
        let session = self.current()?;
        let room_id = session
            .resolve_room_id(input)
            .await
            .map_err(|error| core_error(&error))?;
        if !session.is_joined(&room_id) {
            return Err(CoreError::Unauthorized {
                reason: l10n!(CATALOG, language(), "chat-link-room-not-joined"),
            });
        }
        Ok(room_id.to_string())
    }
}
