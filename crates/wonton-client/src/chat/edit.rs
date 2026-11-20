//! 编辑自己的消息（ADR-0109 第 8 节）：进入编辑态时记下之前的草稿，取消或发成功后原样恢复；
//! 发出去的是那条事件的 `m.replace`。

use std::sync::Arc;

use wonton_l10n::l10n;
use wonton_protocol::{ChatFailure, ChatProblem, CoreError, TimelineAction};

use super::jump::{Editing, Live};
use super::util::lock;
use super::{CATALOG, ChatLink, invalid, language};

impl ChatLink {
    /// 这个订阅里能编辑的那个条目的事件 ID 和原文。
    fn menu_edit(
        &self,
        key: &super::TimelineKey,
        item_id: &str,
    ) -> Result<(String, String), CoreError> {
        let open = lock(&self.open);
        if open.get(key).is_none() {
            return Err(super::target::not_subscribed(key));
        }
        let found = open
            .matching(key)
            .filter_map(|timeline| timeline.menus.upgrade())
            .find_map(|menus| lock(&menus).edit(item_id));
        found.ok_or_else(|| {
            invalid(&l10n!(
                CATALOG,
                language(),
                "chat-message-action-unavailable"
            ))
        })
    }

    /// 订阅里编辑态用的那份状态。
    fn live(&self, key: &super::TimelineKey) -> Result<Arc<Live>, CoreError> {
        let subscription = self.subscription(key)?;
        lock(&subscription.live)
            .clone()
            .ok_or_else(|| super::target::not_subscribed(key))
    }

    /// 进入编辑态：原文进输入框，进入之前的草稿先读出来，退出时恢复。
    pub(super) fn start_edit(
        self: &Arc<Self>,
        key: &super::TimelineKey,
        item_id: &str,
    ) -> Result<(), CoreError> {
        let (event_id, text) = self.menu_edit(key, item_id)?;
        let event_id: wonton_matrix::OwnedEventId = event_id
            .parse()
            .unwrap_or_else(|_| unreachable!("menu event id"));
        let session = self.current()?;
        let live = self.live(key)?;
        let chat = self.clone();
        let item_id = item_id.to_owned();
        let key = key.clone();
        self.runtime.spawn(async move {
            let previous = match session.draft(&key.room_id, key.thread()).await {
                Ok(Some(previous)) => previous,
                Ok(None) => wonton_matrix::Draft {
                    body: String::new(),
                    reply_to: None,
                },
                Err(error) => {
                    chat.report_op_failure("chat-op-edit-failed", &error);
                    return;
                }
            };
            {
                let mut editing = lock(&live.editing);
                // 已经编辑着一条又点了另一条：换成新的一条，进入编辑前的草稿仍留着。
                let previous = editing
                    .as_ref()
                    .map(|editing| editing.previous.clone())
                    .unwrap_or(previous);
                *editing = Some(Editing {
                    item_id,
                    event_id,
                    body: text.clone(),
                    previous,
                });
            }
            if let Err(error) = chat.set_draft(&key, &text, None) {
                report(&chat, &error);
            }
            live.edit_changed();
        });
        Ok(())
    }

    /// 退出编辑态，把进入前的草稿和回复对象原样放回去。
    pub(super) fn cancel_edit(self: &Arc<Self>, key: &super::TimelineKey) -> Result<(), CoreError> {
        let live = self.live(key)?;
        let Some(editing) = lock(&live.editing).take() else {
            return Err(invalid(&l10n!(
                CATALOG,
                language(),
                "chat-message-action-unavailable"
            )));
        };
        self.stop_typing(key);
        let reply = editing
            .previous
            .reply_to
            .map(|event_id| event_id.to_string());
        if let Err(error) = self.set_draft(key, &editing.previous.body, reply.as_deref()) {
            report(self, &error);
        }
        // 草稿先放回去再通知，壳收到编辑结束去读草稿时读到的是恢复后的。
        live.edit_changed();
        Ok(())
    }

    /// 确认：新正文发到原来那条事件上。失败时编辑态和输入框都留着。
    pub(super) fn edit_message(
        self: &Arc<Self>,
        key: &super::TimelineKey,
        event_id: &str,
        body: &str,
    ) -> Result<(), CoreError> {
        let body = body.trim().to_owned();
        if body.is_empty() {
            return Err(invalid(&l10n!(CATALOG, language(), "chat-empty-message")));
        }
        let live = self.live(key)?;
        {
            let editing = lock(&live.editing);
            let Some(editing) = editing.as_ref() else {
                return Err(invalid(&l10n!(
                    CATALOG,
                    language(),
                    "chat-message-action-unavailable"
                )));
            };
            if editing.event_id.as_str() != event_id {
                return Err(invalid(&l10n!(
                    CATALOG,
                    language(),
                    "chat-message-action-unavailable"
                )));
            }
        }
        let event_id: wonton_matrix::OwnedEventId = event_id.parse().map_err(|_| {
            invalid(&l10n!(
                CATALOG,
                language(),
                "chat-message-action-unavailable"
            ))
        })?;
        let chat = self.clone();
        let op_chat = self.clone();
        let key = key.clone();
        let op_key = key.clone();
        self.spawn_op_with_success(
            "chat-op-edit-failed",
            async move {
                let room = op_chat.room(&op_key).await?;
                room.edit_message(&event_id, &body).await
            },
            move || {
                chat.stop_typing(&key);
                let Some(editing) = lock(&live.editing).take() else {
                    return;
                };
                let reply = editing
                    .previous
                    .reply_to
                    .map(|event_id| event_id.to_string());
                if let Err(error) = chat.set_draft(&key, &editing.previous.body, reply.as_deref()) {
                    report(&chat, &error);
                }
                live.edit_changed();
            },
        );
        Ok(())
    }

    /// 壳报告正在输入，或停下（[`TimelineAction::SetTyping`]）。
    pub(super) fn set_typing(
        self: &Arc<Self>,
        key: &super::TimelineKey,
        typing: bool,
    ) -> Result<(), CoreError> {
        if let Some(reporter) = self.live(key)?.reporter.clone() {
            reporter.set(typing);
        }
        Ok(())
    }

    /// 发出去一条消息了，停下正在输入；没有订阅就不管。
    pub(super) fn stop_typing(self: &Arc<Self>, key: &super::TimelineKey) {
        let Ok(live) = self.live(key) else { return };
        if let Some(reporter) = &live.reporter {
            reporter.stop();
        }
    }

    /// 编辑和正在输入四件事按 `action` 分派，`timeline_action` 只留一处入口。
    pub(super) fn editing_action(
        self: &Arc<Self>,
        action: TimelineAction,
    ) -> Result<(), CoreError> {
        match action {
            TimelineAction::StartEdit { target, item_id } => {
                self.start_edit(&super::TimelineKey::parse(&target)?, &item_id)
            }
            TimelineAction::CancelEdit { target } => {
                self.cancel_edit(&super::TimelineKey::parse(&target)?)
            }
            TimelineAction::EditMessage {
                target,
                event_id,
                body,
            } => self.edit_message(&super::TimelineKey::parse(&target)?, &event_id, &body),
            TimelineAction::SetTyping { target, typing } => {
                self.set_typing(&super::TimelineKey::parse(&target)?, typing)
            }
            _ => unreachable!("only edit and typing reach here"),
        }
    }
}

/// 本机草稿读写失败进 `problem`：和后台操作一样，只是原因不出在服务器上。
fn report(chat: &ChatLink, error: &CoreError) {
    tracing::info!(message_id = "chat-op-edit-failed", error = %error, "chat action failed");
    let reason = match error {
        CoreError::TransportUnavailable { .. } => ChatFailure::TransportUnavailable,
        CoreError::Unauthorized { .. } => ChatFailure::Unauthorized,
        CoreError::Forbidden { .. } => ChatFailure::Forbidden,
        CoreError::NotReady { .. } => ChatFailure::NotReady,
        CoreError::InvalidInput { .. } => ChatFailure::InvalidInput,
        _ => ChatFailure::Upstream,
    };
    let message = match reason {
        ChatFailure::TransportUnavailable => super::view::not_connected(),
        _ => l10n!(
            CATALOG,
            language(),
            "chat-op-edit-failed",
            error = error.to_string()
        ),
    };
    chat.view
        .send_modify(|view| view.problem = Some(ChatProblem { reason, message }));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn editing(body: &str) -> Editing {
        Editing {
            item_id: "i".to_owned(),
            event_id: "$a:home.example".parse().unwrap(),
            body: body.to_owned(),
            previous: wonton_matrix::Draft {
                body: String::new(),
                reply_to: None,
            },
        }
    }

    /// 编辑条上的摘要只有一行、不带换行，长短消息都不撑破。
    #[test]
    fn the_summary_stays_on_one_line() {
        assert_eq!(editing("  改  \n 一下 ").summary(), "改 一下");
        let long = "字".repeat(200);
        let summary = editing(&long).summary();
        assert!(summary.ends_with('…'));
        assert_eq!(summary.chars().count(), 121);
    }
}
