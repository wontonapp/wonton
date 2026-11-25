//! 编辑消息和正在输入（ADR-0109 第 8、10 节）：壳的 `StartEdit`、`CancelEdit`、`EditMessage`、`SetTyping` 落在核心这一侧。
//!
//! 编辑态在订阅上，换了房间（订阅释放）就没了；原草稿在进入编辑态时存下来，取消和发送成功后原样恢复；编辑途中又打了字就不盖。

use std::sync::Arc;

use wonton_l10n::l10n;
use wonton_matrix::Draft;
use wonton_protocol::{CoreError, TimelineTarget};

use super::draft::DraftWrite;
use super::{ChatLink, TimelineKey, invalid, lock};
use crate::l10n::{CATALOG, language};

impl ChatLink {
    /// 这条订阅的编辑态和输入状态。房间没订阅时间线是未就绪。
    fn live(&self, key: &TimelineKey) -> Result<Arc<super::jump::Live>, CoreError> {
        let subscription = self.subscription(key)?;
        let live = lock(&subscription.live).as_ref().cloned();
        live.ok_or_else(|| invalid(&l10n!(CATALOG, language(), "chat-invalid-action")))
    }

    /// 进入编辑态：存下现在的草稿，换成这条的正文。壳已经从 [`MessageAction::Edit`](wonton_protocol::MessageAction::Edit)
    /// 拿到了正文，这里只接受菜单里还有这一项的消息。
    pub(super) fn start_edit(
        self: &Arc<Self>,
        key: &TimelineKey,
        item_id: &str,
    ) -> Result<(), CoreError> {
        let (event_id, text) = self.menu_edit(key, item_id)?;
        let live = self.live(key)?;
        // 进编辑态先停“正在输入”，壳填完字会再报。
        live.typing_out.set(false);
        let token = live.edit.begin_token();
        // 已经在编辑别的消息时，恢复用的仍是第一次进来之前那份，不拿现在输入框里的正文。
        let carried = live.edit.saved();
        let chat = self.clone();
        let key = key.clone();
        let item_id = item_id.to_owned();
        self.spawn_op("chat-op-edit-failed", async move {
            let saved = match carried {
                Some(saved) => saved,
                // 读不回原草稿时说一声，别让取消把主人的草稿悄悄换成空。
                None => match chat.read_raw_draft(&key).await {
                    Ok(Some(saved)) => saved,
                    Ok(None) => Draft {
                        body: String::new(),
                        reply_to: None,
                    },
                    Err(error) => {
                        let error = wonton_matrix::Error::NotReady(error.to_string());
                        chat.report_op_failure("chat-op-edit-failed", &error);
                        Draft {
                            body: String::new(),
                            reply_to: None,
                        }
                    }
                },
            };
            live.edit.begin(
                token,
                item_id,
                event_id.to_string(),
                &text,
                saved,
                |draft| {
                    chat.enqueue_draft(DraftWrite::Save(key, draft));
                },
            );
            Ok(())
        });
        Ok(())
    }

    /// 退出编辑态：恢复进入编辑态之前的草稿。没在编辑时什么也不做。
    pub(super) fn cancel_edit(&self, key: &TimelineKey) -> Result<(), CoreError> {
        let live = self.live(key)?;
        live.typing_out.set(false);
        let Some((_event_id, editing, saved)) = live.edit.take() else {
            return Ok(());
        };
        self.enqueue_draft(DraftWrite::Restore(key.clone(), editing, saved));
        Ok(())
    }

    /// 确认这次修改：发 `m.replace`，成功后清掉编辑态、恢复原草稿；失败时编辑态和输入框留着。
    pub(super) fn edit_message(
        self: &Arc<Self>,
        key: &TimelineKey,
        event_id: &str,
        body: &str,
    ) -> Result<(), CoreError> {
        if body.trim().is_empty() {
            return Err(invalid(&l10n!(CATALOG, language(), "chat-empty-message")));
        }
        let live = self.live(key)?;
        if !live.edit.is_editing(event_id) {
            return Err(invalid(&l10n!(
                CATALOG,
                language(),
                "chat-message-action-unavailable"
            )));
        }
        let edit = live.edit.clone();
        let chat = self.clone();
        let key = key.clone();
        let event_id = event_id.to_owned();
        let body = body.to_owned();
        live.typing_out.set(false);
        self.spawn_op("chat-op-edit-failed", async move {
            let room = chat.room(&key).await?;
            room.edit_message(&event_id, &body).await?;
            // 发完又去编辑别的了：那一条的编辑态和原草稿留着。
            if edit.is_editing(&event_id)
                && let Some((_sent, editing, saved)) = edit.take()
            {
                chat.enqueue_draft(DraftWrite::Restore(key, editing, saved));
            }
            Ok(())
        });
        Ok(())
    }

    /// 壳报告输入框在不在打字。
    pub(super) fn set_typing(&self, key: &TimelineKey, typing: bool) -> Result<(), CoreError> {
        self.live(key)?.typing_out.set(typing);
        Ok(())
    }

    /// 输入框里要发送了（包括附件区整单）：先收掉这条时间线的“正在输入”。附件区那条发送路不走 `timeline_action`，
    /// 由 `dispatch` 统一在发送前叫，真路径和假数据都一样。
    pub(crate) fn stop_typing(&self, target: &TimelineTarget) {
        if let Ok(key) = TimelineKey::parse(target) {
            #[cfg(all(test, feature = "ui-fake"))]
            lock(&self.typing_stops).push(key.clone());
            if let Ok(live) = self.live(&key) {
                live.typing_out.set(false);
            }
        }
    }
}
