//! 转发和撤回（ADR-0073 第 4、5 节）：只对菜单里有这一项的消息做，失败进 `ChatViewModel::problem`。

use std::sync::Arc;

use wonton_l10n::l10n;
use wonton_matrix::{Error, SendOutcome};
use wonton_protocol::{CoreError, MessageAction};

use super::{ChatLink, TimelineKey, invalid, onboarding};
use crate::l10n::{CATALOG, language};

impl ChatLink {
    /// 照原样发到另一个已加入的房间，进展在那个房间的时间线里。
    pub(super) fn forward_message(
        self: &Arc<Self>,
        key: &TimelineKey,
        item_id: &str,
        to_room_id: &str,
    ) -> Result<(), CoreError> {
        let room = self.opened(key)?;
        let event_id = self.menu_event(key, item_id, |action| {
            matches!(action, MessageAction::Forward)
        })?;
        let (session, to) = self.send_target(to_room_id)?;
        if !session.is_joined(&to) {
            return Err(invalid(&l10n!(
                CATALOG,
                language(),
                "chat-forward-target-not-joined"
            )));
        }
        // 这个房间的上一条文字（也就是这次转发的附言）还没被服务器收下。等它有了结果：收下了才发转发，
        // 不可恢复地失败就不发，让视图把失败如实报出来。
        let order = self
            .forward_orders
            .lock()
            .unwrap()
            .remove(to.as_str())
            .map(|sender| sender.subscribe());
        let sent = self.sent.clone();
        let sent_room = to.to_string();
        self.spawn_op_with_success(
            "chat-op-forward-failed",
            async move {
                if let Some(mut order) = order {
                    let outcome = order
                        .wait_for(|outcome| outcome.is_some())
                        .await
                        .ok()
                        .and_then(|outcome| outcome.clone());
                    match outcome {
                        Some(SendOutcome::Sent) => {}
                        Some(SendOutcome::Failed(error)) => {
                            return Err(Error::Upstream(l10n!(
                                CATALOG,
                                language(),
                                "chat-forward-comment-failed",
                                error = error
                            )));
                        }
                        None => {
                            return Err(Error::Upstream(l10n!(
                                CATALOG,
                                language(),
                                "chat-forward-comment-failed",
                                error = l10n!(CATALOG, language(), "chat-forward-comment-lost")
                            )));
                        }
                    }
                }
                session.forward(&room, &event_id, &to).await
            },
            move || onboarding::record_sent(&sent, sent_room),
        );
        Ok(())
    }

    /// 撤回：所有人那里都变成已撤回。没权限、连不上、服务器拒绝分别按原因进 `problem`。
    pub(super) fn redact_message(&self, key: &TimelineKey, item_id: &str) -> Result<(), CoreError> {
        let room = self.opened(key)?;
        let event_id = self.menu_event(key, item_id, |action| {
            matches!(action, MessageAction::Redact)
        })?;
        self.spawn_op("chat-op-redact-failed", async move {
            room.redact_message(&event_id).await
        });
        Ok(())
    }
}
