//! 输入框里的草稿与编辑、发出去和转发回来的回显。

use wonton_protocol::{
    ComposerEdit, MessageAction, MessageContent, MessageStatus, QuotedContent, QuotedMessage,
    ReplyQuote, RichBlock, RichBlockKind, RichSpan, TimelineContent, TimelineDiff, TimelineItem,
};

use super::timeline::{Editing, FakeTimeline, Saved};
use super::ui_fake_room::{ITEMS, OWNER, at_ms, text_message};

/// 退出编辑态换回原草稿：编辑途中又打了字（输入框里已不是原文）就留着新打的。
fn restore_after_edit(draft: &mut Saved, editing: &Editing) {
    if draft.body.trim() == editing.text.trim() {
        *draft = editing.saved.clone();
    }
}

impl FakeTimeline {
    /// 假数据收下一张照片：补一条写着 photo（有说明就写说明）的文字，回复对象原样带上。真上传和媒体句柄不管。
    pub(super) fn sent_photo(&self, reply_to: Option<&str>, caption: Option<&str>) {
        let index = {
            let mut state = self.lock();
            state.sent += 1;
            ITEMS - 1 + state.sent
        };
        let id = format!("ui-fake-photo-{index}");
        let body = caption
            .map(str::trim)
            .filter(|c| !c.is_empty())
            .unwrap_or("photo");
        let mut message = text_message(&id, OWNER, at_ms(index), &[body.to_owned()]);
        message.status = Some(MessageStatus::Sent);
        message.reply = reply_to.and_then(|event| {
            let text = self.text_of(event.trim_start_matches('$'))?;
            Some(ReplyQuote::Ready {
                message: QuotedMessage {
                    event_id: event.to_owned(),
                    sender_id: OWNER.into(),
                    sender_name: "Owner".into(),
                    content: QuotedContent::Text { text },
                },
            })
        });
        let item = TimelineItem {
            id,
            content: TimelineContent::Message { message },
        };
        self.batch(|update| {
            update.diffs = vec![TimelineDiff::PushBack { item }];
            update.scroll_to_bottom = true;
        });
    }

    /// 自己发的一条接在末尾，比上一条晚一分钟（和上一条自己的消息连成一组）。
    /// 提及的用户不画胶囊：假数据里没有成员表，胶囊的样式由真实房间里看。
    pub(super) fn send_text(&self, body: &str, _mentions: &[String]) {
        let (index, sent) = {
            let mut state = self.lock();
            state.sent += 1;
            (ITEMS - 1 + state.sent, state.sent)
        };
        let mut message = text_message(
            &format!("ui-fake-sent-{sent}"),
            OWNER,
            at_ms(index),
            &[body.to_owned()],
        );
        message.status = Some(MessageStatus::Sent);
        let item = TimelineItem {
            id: format!("ui-fake-sent-{sent}"),
            content: TimelineContent::Message { message },
        };
        self.batch(|update| {
            update.diffs = vec![TimelineDiff::PushBack { item }];
            update.scroll_to_bottom = true;
        });
    }

    /// 壳把输入框里的字交上来。空空如也时按没有草稿算。
    pub(super) fn set_draft(&self, body: &str, reply_to: Option<String>) {
        let mut state = self.lock();
        state.draft = Saved {
            body: body.to_owned(),
            reply_to,
        };
    }

    /// 进入编辑态：存下现在的草稿，输入框换成原文（真核心在 `chat/edit_actions.rs` 做同样的事）。
    /// 菜单里没有这条时不动。
    pub(super) fn start_edit(&self, item_id: &str) {
        let mut state = self.lock();
        let found = state.items.iter().find_map(|item| {
            let TimelineContent::Message { message } = &item.content else {
                return None;
            };
            if item.id != item_id {
                return None;
            }
            let MessageContent::Text { body, .. } = &message.body else {
                return None;
            };
            Some((message.event_id.clone()?, body.clone()))
        });
        let Some((event_id, text)) = found else {
            return;
        };
        // 已经在编辑别的消息：原草稿仍是第一次进来之前那份，不拿现在的输入框。
        let saved = match state.editing.take() {
            Some(previous) => previous.saved,
            None => std::mem::take(&mut state.draft),
        };
        state.editing = Some(Editing {
            card: ComposerEdit {
                item_id: item_id.to_owned(),
                event_id,
                summary: summary(&text),
            },
            text: text.clone(),
            saved,
        });
        state.draft = Saved {
            body: text,
            reply_to: None,
        };
        drop(state);
        self.batch(|_| {});
    }

    /// 退出编辑态：草稿换回进入编辑态之前的。没在编辑时什么也不做。
    pub(super) fn cancel_edit(&self) {
        let mut state = self.lock();
        let Some(editing) = state.editing.take() else {
            return;
        };
        restore_after_edit(&mut state.draft, &editing);
        drop(state);
        self.batch(|_| {});
    }

    /// 确认这次修改：这一条换成新正文并标“已编辑”，草稿换回进入编辑态之前的。
    pub(super) fn edit_message(&self, event_id: &str, body: &str) {
        if body.trim().is_empty() {
            return;
        }
        let edited = {
            let mut state = self.lock();
            let Some(editing) = state.editing.as_ref() else {
                return;
            };
            if editing.card.event_id != event_id {
                return;
            }
            let found = state.items.iter().enumerate().find_map(|(index, item)| {
                let TimelineContent::Message { message } = &item.content else {
                    return None;
                };
                if message.event_id.as_deref() != Some(event_id) {
                    return None;
                }
                let mut item = item.clone();
                let TimelineContent::Message { message } = &mut item.content else {
                    return None;
                };
                let MessageContent::Text {
                    body: current,
                    blocks,
                    ..
                } = &mut message.body
                else {
                    return None;
                };
                *current = body.to_owned();
                *blocks = paragraphs(body);
                message.edited = true;
                // 菜单跟着换：改过的还能再改，复制拿到的也是新正文。
                for action in &mut message.actions {
                    match action {
                        MessageAction::Copy { text } | MessageAction::Edit { text } => {
                            *text = body.to_owned();
                        }
                        _ => {}
                    }
                }
                Some((index, item))
            });
            let Some(found) = found else {
                return;
            };
            let editing = state.editing.take().unwrap_or_else(|| unreachable!());
            restore_after_edit(&mut state.draft, &editing);
            found
        };
        let (index, item) = edited;
        self.batch(|update| {
            update.diffs = vec![TimelineDiff::Set {
                index: u32::try_from(index).unwrap_or(u32::MAX),
                item,
            }];
        });
    }
}

/// 编辑条上的一行摘要：第一个非空行，去掉首尾空白，太长截断（同 `chat/edit.rs`）。
fn summary(text: &str) -> String {
    let Some(line) = text.lines().map(str::trim).find(|line| !line.is_empty()) else {
        return String::new();
    };
    if line.chars().count() <= 80 {
        return line.to_owned();
    }
    let mut cut: String = line.chars().take(80).collect();
    cut.push('…');
    cut
}

fn paragraphs(body: &str) -> Vec<RichBlock> {
    body.split("\n\n")
        .map(|text| RichBlock {
            kind: RichBlockKind::Paragraph,
            depth: 0,
            quoted: false,
            spans: vec![RichSpan {
                text: text.to_owned(),
                ..RichSpan::default()
            }],
        })
        .collect()
}

#[cfg(test)]
#[path = "compose_tests.rs"]
mod compose_tests;
