//! 壳发来的时间线和选项动作：按目标找回时间线，转发、跳转、草稿和编辑各走各的。

use wonton_protocol::{
    ChatAction, ChoiceSend, ChoicesState, TimelineAction, TimelineContent, TimelineDiff,
    TimelineTarget,
};

use super::lock;
use super::subscriptions::FakeTimelines;
use super::timeline::FakeTimeline;

impl FakeTimelines {
    /// 时间线上的操作。没订阅的房间、约定以外的操作不理。
    pub(crate) fn timeline_action(&self, action: TimelineAction) {
        if let TimelineAction::ForwardMessage {
            target,
            item_id,
            to_room_id,
            comment,
        } = &action
        {
            // 附言先落进目标房间再转发，和真核心等附言被确认后才转发的顺序一致。
            if let Some(comment) = comment.as_deref().map(str::trim).filter(|c| !c.is_empty()) {
                let to = TimelineTarget {
                    room_id: to_room_id.clone(),
                    thread_root: None,
                };
                match self.get(&to) {
                    Some(fake) => fake.send_text(comment, &[]),
                    None => self.park(to_room_id, comment),
                }
            }
            self.forward(target, item_id, to_room_id);
            return;
        }
        let target = match &action {
            TimelineAction::JumpToMessage { target, .. }
            | TimelineAction::RegisterFocusRevision { target, .. }
            | TimelineAction::JumpBack { target }
            | TimelineAction::StartEdit { target, .. }
            | TimelineAction::CancelEdit { target }
            | TimelineAction::EditMessage { target, .. }
            | TimelineAction::SetDraft { target, .. }
            | TimelineAction::SetTimelineScroll { target, .. }
            | TimelineAction::SendText { target, .. }
            | TimelineAction::SendImage { target, .. } => target,
            _ => return,
        };
        let Some(fake) = self.get(target) else {
            // 转发连同附言常常发往还没打开的房间：留着，等它打开时和转发的消息一起补进第一批。
            if let TimelineAction::SendText { body, .. } = &action
                && !body.is_empty()
            {
                lock(&self.forwarded)
                    .entry(target.room_id.clone())
                    .or_default()
                    .push(body.clone());
            }
            return;
        };
        match action {
            TimelineAction::JumpToMessage {
                event_id,
                from_item_id,
                ..
            } => fake.jump_to(&event_id, from_item_id),
            TimelineAction::JumpBack { .. } => fake.jump_back(),
            TimelineAction::RegisterFocusRevision { revision, .. } => {
                fake.register_revision(revision)
            }
            TimelineAction::SetTimelineScroll { scroll, .. } => fake.scrolled(scroll),
            // 只发了照片、没写附言时正文是空的，假时间线不画空的一行。
            TimelineAction::SendText { body, mentions, .. } if !body.is_empty() => {
                fake.send_text(&body, &mentions)
            }
            // 假数据没有真上传：照片也补一条，回复对象和说明照原样带上，界面测试才看得见第一张带没带回复。
            TimelineAction::SendImage {
                reply_to, caption, ..
            } => fake.sent_photo(reply_to.as_deref(), caption.as_deref()),
            TimelineAction::SetDraft { body, reply_to, .. } => fake.set_draft(&body, reply_to),
            TimelineAction::StartEdit { item_id, .. } => fake.start_edit(&item_id),
            TimelineAction::CancelEdit { .. } => fake.cancel_edit(),
            TimelineAction::EditMessage { event_id, body, .. } => {
                fake.edit_message(&event_id, &body)
            }
            _ => {}
        }
    }

    /// 点了选项按钮：这个选项选上，整组变成已选。
    pub(crate) fn chat_action(&self, action: &ChatAction) {
        let ChatAction::AnswerAgentChoice {
            room_id,
            item_id,
            key,
        } = action
        else {
            return;
        };
        let target = TimelineTarget {
            room_id: room_id.clone(),
            thread_root: None,
        };
        if let Some(fake) = self.get(&target) {
            fake.answer(item_id, key);
        }
    }
}

impl FakeTimeline {
    pub(super) fn answer(&self, item_id: &str, key: &str) {
        let answered = {
            let state = self.lock();
            state.items.iter().enumerate().find_map(|(index, item)| {
                let TimelineContent::Message { message } = &item.content else {
                    return None;
                };
                let choices = message.choices.as_ref()?;
                if item.id != item_id
                    || choices.state != ChoicesState::Open
                    || !choices.options.iter().any(|option| option.key == key)
                {
                    return None;
                }
                let mut item = item.clone();
                if let TimelineContent::Message { message } = &mut item.content
                    && let Some(choices) = &mut message.choices
                {
                    choices.state = ChoicesState::Answered;
                    for option in &mut choices.options {
                        option.chosen = option.key == key;
                        option.send = ChoiceSend::Idle;
                    }
                }
                Some((index, item))
            })
        };
        let Some((index, item)) = answered else {
            return;
        };
        self.batch(|update| {
            update.diffs = vec![TimelineDiff::Set {
                index: u32::try_from(index).unwrap_or(u32::MAX),
                item,
            }];
        });
    }
}

#[cfg(test)]
#[path = "actions_tests.rs"]
mod actions_tests;
