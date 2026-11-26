//! 发送方的上限和阻塞等待者。

use super::*;

impl Inbox {
    pub(super) fn check_limits(
        &self,
        inner: &Inner,
        sender_key: &str,
        kind: InboxType,
        now: u64,
    ) -> Result<(), InboxError> {
        if let Some(page) = pending_class(kind) {
            let (items, pages) = pending_counts(inner, sender_key);
            if page && pages >= MAX_PENDING_INBOX_PAGES {
                return Err(InboxError::TooManyPending);
            }
            if !page && items >= MAX_PENDING_INBOX_ITEMS {
                return Err(InboxError::TooManyPending);
            }
        }
        let mut rate = inner.rate.get(sender_key).cloned().unwrap_or_default();
        rate.retain(|at| now.saturating_sub(*at) <= DAY_MS);
        let minute = rate
            .iter()
            .filter(|at| now.saturating_sub(**at) <= MINUTE_MS)
            .count();
        if minute >= MAX_INBOX_PER_MINUTE || rate.len() >= MAX_INBOX_PER_DAY {
            return Err(InboxError::RateLimited);
        }
        Ok(())
    }

    pub(super) fn record_create_rate(&self, sender_key: &str, now: u64) {
        let mut inner = self.lock();
        inner
            .rate
            .entry(sender_key.to_owned())
            .or_default()
            .push_back(now);
    }

    /// 阻塞地等某条条目的答案；只有发送方能等，别的 Bot 知道 id 也读不到。
    pub async fn wait_for_sender(
        &self,
        sender_id: &str,
        id: &str,
        after_revision: Option<u64>,
    ) -> Result<InboxItem, InboxError> {
        let mut revision = self.subscribe();
        loop {
            let now = self.now();
            {
                let inner = self.lock();
                let record = inner.records.get(id).ok_or(InboxError::NotFound)?;
                let item = &record.stored.item;
                if item.sender.id != sender_id {
                    return Err(InboxError::NotYours);
                }
                let reached = matches!(
                    item.status,
                    InboxStatus::Handled
                        | InboxStatus::HandledElsewhere
                        | InboxStatus::Expired
                        | InboxStatus::Withdrawn
                        | InboxStatus::Archived
                );
                if reached || after_revision.is_none_or(|after| inner.revision > after) {
                    let _ = now;
                    return Ok(item.clone());
                }
            }
            if revision.changed().await.is_err() {
                return Err(InboxError::ServiceUnavailable);
            }
        }
    }
}
