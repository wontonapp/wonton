//! 时间线的发送动作：重发、取消发送、往前翻页，以及 `TimelineAction` 的分派。
//!
//! 往前翻页一次加载的事件数也在这儿：这里的翻页和 `jump` 的往前翻都读它。

use std::sync::Arc;

use wonton_l10n::l10n;
use wonton_protocol::{CoreError, TimelineAction, TimelineTarget};

use super::onboarding;
use super::util::invalid;
use super::{ChatLink, TimelineKey};
use crate::l10n::{CATALOG, language};

/// 向前翻页一次最多加载的事件数。
pub(super) const PAGE: u16 = 30;

impl ChatLink {
    fn retry_send(
        self: &Arc<Self>,
        target: &TimelineTarget,
        item_id: &str,
    ) -> Result<(), CoreError> {
        let key = TimelineKey::parse(target)?;
        let room = self.opened(&key)?;
        let sent = self.sent.clone();
        let item_id = item_id.to_owned();
        self.spawn_op_with_success(
            |_| "chat-op-resend-failed",
            async move { room.retry_send(&item_id).await },
            move || onboarding::record_sent(&sent, key.room_id.to_string()),
        );
        Ok(())
    }

    fn cancel_send(&self, target: &TimelineTarget, item_id: &str) -> Result<(), CoreError> {
        let room = self.opened(&TimelineKey::parse(target)?)?;
        let item_id = item_id.to_owned();
        self.spawn_op("chat-op-cancel-send-failed", async move {
            room.cancel_send(&item_id).await
        });
        Ok(())
    }

    fn paginate_backwards(&self, target: &TimelineTarget) -> Result<(), CoreError> {
        let room = self.opened(&TimelineKey::parse(target)?)?;
        self.spawn_op("chat-op-paginate-failed", async move {
            room.paginate_backwards(PAGE).await.map(drop)
        });
        Ok(())
    }

    #[allow(clippy::too_many_lines, reason = "dispatch table")]
    pub(crate) fn timeline_action(
        self: &Arc<Self>,
        action: TimelineAction,
    ) -> Result<(), CoreError> {
        match action {
            TimelineAction::SendText {
                target,
                body,
                reply_to,
                mentions,
                ..
            } => self.send_text(
                &TimelineKey::parse(&target)?,
                &body,
                reply_to.as_deref(),
                &mentions,
            ),
            TimelineAction::SendImage {
                target,
                image,
                caption,
                reply_to,
            } => self.send_image(
                &TimelineKey::parse(&target)?,
                image,
                caption.as_deref(),
                reply_to.as_deref(),
            ),
            TimelineAction::ForwardMessage {
                target,
                item_id,
                to_room_id,
                comment,
            } => self.forward_message(
                &TimelineKey::parse(&target)?,
                &item_id,
                &to_room_id,
                comment.as_deref(),
            ),
            TimelineAction::RedactMessage { target, item_id } => {
                self.redact_message(&TimelineKey::parse(&target)?, &item_id)
            }
            TimelineAction::RetrySend { target, item_id } => self.retry_send(&target, &item_id),
            TimelineAction::CancelSend { target, item_id } => self.cancel_send(&target, &item_id),
            TimelineAction::PaginateBackwards { target } => self.paginate_backwards(&target),
            TimelineAction::SetTimelineScroll { target, scroll } => {
                self.set_timeline_scroll(&TimelineKey::parse(&target)?, scroll)
            }
            TimelineAction::MarkRead { target } => {
                self.mark_room_read(&TimelineKey::parse(&target)?)
            }
            TimelineAction::SetDraft {
                target,
                body,
                reply_to,
            } => self.set_draft(&TimelineKey::parse(&target)?, &body, reply_to.as_deref()),
            TimelineAction::DismissLinkPreview { target } => {
                self.dismiss_preview(TimelineKey::parse(&target)?);
                Ok(())
            }
            // 跳转的两步只动订阅里的状态：找到就发焦点、找不到就往前翻页（ADR-0109 §4）。
            TimelineAction::JumpToMessage {
                target,
                event_id,
                from_item_id,
            } => {
                let subscription = self.subscription(&TimelineKey::parse(&target)?)?;
                let revision = subscription.jumps_revision();
                subscription.jump_to(&event_id, &from_item_id, revision);
                Ok(())
            }
            TimelineAction::RegisterFocusRevision { target, revision } => {
                self.subscription(&TimelineKey::parse(&target)?)?
                    .register_focus_revision(revision);
                Ok(())
            }
            TimelineAction::JumpBack { target } => {
                self.subscription(&TimelineKey::parse(&target)?)?
                    .jump_back();
                Ok(())
            }
            // 附件区的操作由 `ClientCore::dispatch` 交给附件区（ADR-0094），不会到这里。
            TimelineAction::SendComposer { .. }
            | TimelineAction::RemoveAttachment { .. }
            | TimelineAction::RetryAttachment { .. }
            | TimelineAction::RetryOutgoing { .. }
            | TimelineAction::CancelOutgoing { .. } => {
                Err(invalid(&l10n!(CATALOG, language(), "chat-invalid-action")))
            }
            TimelineAction::StartEdit { target, item_id } => {
                self.start_edit(&TimelineKey::parse(&target)?, &item_id)
            }
            TimelineAction::CancelEdit { target } => {
                self.cancel_edit(&TimelineKey::parse(&target)?)
            }
            TimelineAction::EditMessage {
                target,
                event_id,
                body,
            } => self.edit_message(&TimelineKey::parse(&target)?, &event_id, &body),
            TimelineAction::SetTyping { target, typing } => {
                self.set_typing(&TimelineKey::parse(&target)?, typing)
            }
            TimelineAction::JumpToFirstUnread { target } => {
                self.jump_to_unread(&TimelineKey::parse(&target)?, false)
            }
            TimelineAction::JumpToNextMention { target } => {
                self.jump_to_unread(&TimelineKey::parse(&target)?, true)
            }
            TimelineAction::UnreadAnchorSeen { target, item_id } => {
                self.subscription(&TimelineKey::parse(&target)?)?
                    .unread_anchor_seen(item_id);
                Ok(())
            }
        }
    }
}
