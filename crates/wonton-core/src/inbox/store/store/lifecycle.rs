//! 条目的生命周期：作答、撤回、取消、稍后提醒和发送方更新。

use super::*;

impl Inbox {
    /// 作答。幂等：同一个 `answer_id` 返回同一条；异步条目 60 秒内可换答案。
    #[allow(clippy::too_many_lines)]
    pub fn answer(
        &self,
        viewer: &str,
        id: &str,
        input: InboxAnswerInput,
    ) -> Result<AnswerOutcome, InboxError> {
        let now = self.now();
        let mut inner = self.lock();
        let record = inner.records.get_mut(id).ok_or(InboxError::NotFound)?;
        if !self.visible(&record.stored.item, viewer) {
            return Err(InboxError::NotYours);
        }
        let item = &mut record.stored.item;
        if let Some(existing) = &item.answer
            && existing.answer_id == input.answer_id
        {
            return Ok(AnswerOutcome {
                item: item.clone(),
                answer: existing.clone(),
                superseded: false,
            });
        }
        match item.status {
            InboxStatus::Pending => {}
            InboxStatus::Expired => return Err(InboxError::Expired),
            InboxStatus::Withdrawn => return Err(InboxError::Withdrawn),
            InboxStatus::Handled | InboxStatus::HandledElsewhere => {
                if item.blocking {
                    return Err(InboxError::AlreadyAnswered);
                }
                let within = item
                    .answered_at_ms
                    .is_some_and(|at| now.saturating_sub(at) <= INBOX_REANSWER_WINDOW_SECS * 1000);
                if !within {
                    return Err(InboxError::AnswerLocked);
                }
            }
            InboxStatus::Archived | InboxStatus::Other => return Err(InboxError::AlreadyAnswered),
        }
        if item.expires_at_ms.is_some_and(|expires| now >= expires) {
            item.status = InboxStatus::Expired;
            record.stored.terminal_at_ms = Some(now);
            let expired = item.clone();
            let waiters = std::mem::take(&mut record.waiters);
            let csp = record.stored.csp.clone();
            drop(inner);
            self.write(&expired, csp.as_deref())?;
            self.bump();
            for waiter in waiters {
                let _ = waiter.send(InboxAnswer {
                    answer_id: "expired".into(),
                    action: InboxAnswerAction::Acknowledge,
                    answers: Vec::new(),
                    action_id: None,
                    comment: Some("过期".into()),
                    answered_at_ms: now,
                    by: None,
                });
            }
            return Err(InboxError::Expired);
        }
        validate_answer(item, &input).map_err(InboxError::Invalid)?;
        let answer = InboxAnswer {
            answer_id: input.answer_id,
            action: input.action,
            answers: input.answers,
            action_id: input.action_id,
            comment: input.comment,
            answered_at_ms: now,
            by: Some(viewer.to_owned()),
        };
        let mut superseded = false;
        if let Some(old) = item.answer.clone() {
            item.superseded_answers.insert(0, old);
            superseded = true;
        }
        item.answer = Some(answer.clone());
        item.status = InboxStatus::Handled;
        item.outcome = Some(match answer.action {
            InboxAnswerAction::Decline => InboxOutcome::Declined,
            InboxAnswerAction::Acknowledge => InboxOutcome::Acknowledged,
            _ => InboxOutcome::Answered,
        });
        item.answered_by = Some(viewer.to_owned());
        if item.answered_at_ms.is_none() {
            item.answered_at_ms = Some(now);
        }
        record.stored.handled_at_ms.get_or_insert(now);
        let waiters = std::mem::take(&mut record.waiters);
        let stored_item = item.clone();
        let csp = record.stored.csp.clone();
        drop(inner);
        let sender_key = format!(
            "{}:{}",
            sender_tag(&stored_item.sender),
            stored_item.sender.id
        );
        self.record_create_rate(&sender_key, now);
        self.write(&stored_item, csp.as_deref())?;
        self.bump();
        for waiter in waiters {
            let _ = waiter.send(answer.clone());
        }
        Ok(AnswerOutcome {
            item: stored_item,
            answer,
            superseded,
        })
    }

    /// 撤回答案；只有答过的异步条目、60 秒内、本人。
    pub fn withdraw(
        &self,
        viewer: &str,
        id: &str,
        answer_id: &str,
    ) -> Result<InboxItem, InboxError> {
        let now = self.now();
        let mut inner = self.lock();
        let record = inner.records.get_mut(id).ok_or(InboxError::NotFound)?;
        if !self.visible(&record.stored.item, viewer) {
            return Err(InboxError::NotYours);
        }
        let item = &mut record.stored.item;
        if item.blocking {
            return Err(InboxError::AnswerLocked);
        }
        if item.status != InboxStatus::Handled {
            return Err(InboxError::AlreadyAnswered);
        }
        if item.answered_by.as_deref() != Some(viewer) {
            return Err(InboxError::NotYours);
        }
        if !item
            .answered_at_ms
            .is_some_and(|at| now.saturating_sub(at) <= INBOX_REANSWER_WINDOW_SECS * 1000)
        {
            return Err(InboxError::AnswerLocked);
        }
        if item.answer.as_ref().map(|a| a.answer_id.as_str()) != Some(answer_id) {
            return Err(InboxError::NotFound);
        }
        item.status = InboxStatus::Pending;
        item.answered_at_ms = None;
        item.answered_by = None;
        item.outcome = None;
        record.stored.handled_at_ms = None;
        record.stored.terminal_at_ms = None;
        let stored = item.clone();
        let csp = record.stored.csp.clone();
        drop(inner);
        self.write(&stored, csp.as_deref())?;
        self.bump();
        Ok(stored)
    }

    /// 发送方撤回。
    pub fn cancel(
        &self,
        viewer: &str,
        id: &str,
        reason: Option<String>,
    ) -> Result<InboxItem, InboxError> {
        let now = self.now();
        let mut inner = self.lock();
        let record = inner.records.get_mut(id).ok_or(InboxError::NotFound)?;
        if !self.visible(&record.stored.item, viewer) {
            return Err(InboxError::NotYours);
        }
        if record.stored.item.status != InboxStatus::Pending {
            return Err(InboxError::AlreadyAnswered);
        }
        record.stored.item.status = InboxStatus::Withdrawn;
        record.stored.item.result = reason;
        record.stored.terminal_at_ms = Some(now);
        let item = record.stored.item.clone();
        let waiters = std::mem::take(&mut record.waiters);
        let csp = record.stored.csp.clone();
        drop(inner);
        self.write(&item, csp.as_deref())?;
        self.bump();
        for waiter in waiters {
            // 撤回也发一条空答案，阻塞的发送方知道结束了。
            let _ = waiter.send(InboxAnswer {
                answer_id: "withdrawn".into(),
                action: InboxAnswerAction::Acknowledge,
                answers: Vec::new(),
                action_id: None,
                comment: item.result.clone(),
                answered_at_ms: now,
                by: Some(viewer.to_owned()),
            });
        }
        Ok(item)
    }

    /// 稍后提醒：只挪出列表，不延长期限。
    pub fn snooze(
        &self,
        viewer: &str,
        id: &str,
        preset: InboxSnoozePreset,
        timezone_offset_min: Option<i32>,
    ) -> Result<InboxItem, InboxError> {
        let now = self.now();
        let until = snooze_until(preset, now, timezone_offset_min.unwrap_or(0))?;
        let mut inner = self.lock();
        let record = inner.records.get_mut(id).ok_or(InboxError::NotFound)?;
        if !self.visible(&record.stored.item, viewer) {
            return Err(InboxError::NotYours);
        }
        if record.stored.item.status != InboxStatus::Pending {
            return Err(InboxError::AlreadyAnswered);
        }
        record.stored.item.snooze_until_ms = Some(until);
        let item = record.stored.item.clone();
        let csp = record.stored.csp.clone();
        drop(inner);
        self.write(&item, csp.as_deref())?;
        self.bump();
        Ok(item)
    }

    /// 看得到某条条目的发送方本人（不看收件人），撤回/更新用它。
    pub fn visible_to_sender(&self, sender_id: &str, id: &str) -> bool {
        let inner = self.lock();
        inner
            .records
            .get(id)
            .is_some_and(|record| record.stored.item.sender.id == sender_id)
    }

    /// 收件人撤回：按收件人/管理员校验；HTTP 的已读入口用它。
    pub fn cancel_by_viewer(
        &self,
        viewer: &str,
        id: &str,
        reason: Option<String>,
    ) -> Result<InboxItem, InboxError> {
        let inner = self.lock();
        let record = inner.records.get(id).ok_or(InboxError::NotFound)?;
        if !self.visible(&record.stored.item, viewer) {
            return Err(InboxError::NotYours);
        }
        drop(inner);
        self.cancel_raw(id, reason)
    }

    /// 发送方撤回：和 `cancel` 同义，只是按发送方 id 校验。
    pub fn cancel_by_sender(
        &self,
        sender_id: &str,
        id: &str,
        reason: Option<String>,
    ) -> Result<InboxItem, InboxError> {
        if !self.visible_to_sender(sender_id, id) {
            return Err(InboxError::NotYours);
        }
        self.cancel_raw(id, reason)
    }

    fn cancel_raw(&self, id: &str, reason: Option<String>) -> Result<InboxItem, InboxError> {
        let now = self.now();
        let mut inner = self.lock();
        let record = inner.records.get_mut(id).ok_or(InboxError::NotFound)?;
        if record.stored.item.status != InboxStatus::Pending {
            return Err(InboxError::AlreadyAnswered);
        }
        record.stored.item.status = InboxStatus::Withdrawn;
        record.stored.item.result = reason;
        record.stored.terminal_at_ms = Some(now);
        let item = record.stored.item.clone();
        let waiters = std::mem::take(&mut record.waiters);
        let csp = record.stored.csp.clone();
        drop(inner);
        self.write(&item, csp.as_deref())?;
        self.bump();
        let answer = InboxAnswer {
            answer_id: "withdrawn".into(),
            action: InboxAnswerAction::Acknowledge,
            answers: Vec::new(),
            action_id: None,
            comment: item.result.clone(),
            answered_at_ms: now,
            by: None,
        };
        for waiter in waiters {
            let _ = waiter.send(answer.clone());
        }
        Ok(item)
    }

    /// 发送方更新还没答的条目：摘要、进度，不新建。
    pub fn update_by_sender(
        &self,
        sender_id: &str,
        id: &str,
        summary: Option<String>,
        progress: Option<f32>,
    ) -> Result<InboxItem, InboxError> {
        let mut inner = self.lock();
        let record = inner.records.get_mut(id).ok_or(InboxError::NotFound)?;
        if record.stored.item.sender.id != sender_id {
            return Err(InboxError::NotYours);
        }
        if record.stored.item.status != InboxStatus::Pending {
            return Err(InboxError::AlreadyAnswered);
        }
        if let Some(summary) = summary {
            record.stored.item.body = summary;
        }
        if let Some(progress) = progress {
            if !(0.0..=1.0).contains(&progress) {
                return Err(InboxError::Invalid(InboxInvalid::Invalid {
                    field: "progress".into(),
                    value: progress.to_string(),
                    reason: "必须在 0 和 1 之间".into(),
                }));
            }
            record.stored.item.progress = Some(progress);
        }
        let item = record.stored.item.clone();
        let csp = record.stored.csp.clone();
        drop(inner);
        self.write(&item, csp.as_deref())?;
        self.bump();
        Ok(item)
    }
}
