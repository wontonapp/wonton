//! 到期、归档和过期条目的清理。

use super::*;

impl Inbox {
    /// 把到期的标记成过期、过保留期的归档/删掉。后台每 30 秒跑一次；测试直接调。
    pub fn sweep(&self, now: u64) {
        let mut inner = self.lock();
        let mut changed: Vec<(InboxItem, Option<String>)> = Vec::new();
        inner.records.retain(|id, record| {
            let item = &mut record.stored.item;
            if item.status == InboxStatus::Pending
                && item.expires_at_ms.is_some_and(|expires| now >= expires)
            {
                item.status = InboxStatus::Expired;
                record.stored.terminal_at_ms = Some(now);
                changed.push((item.clone(), record.stored.csp.clone()));
                return true;
            }
            if item.status == InboxStatus::Handled
                && record
                    .stored
                    .handled_at_ms
                    .is_some_and(|at| now.saturating_sub(at) > DAY_MS)
            {
                item.status = InboxStatus::Archived;
                record.stored.terminal_at_ms = Some(now);
                changed.push((item.clone(), record.stored.csp.clone()));
                return true;
            }
            if item.status == InboxStatus::Archived
                && record
                    .stored
                    .terminal_at_ms
                    .is_some_and(|at| now.saturating_sub(at) > 30 * DAY_MS)
            {
                let _ = std::fs::remove_file(self.dir.join(PAGES_DIR).join(format!("{id}.html")));
                let _ = std::fs::remove_file(self.dir.join(ITEMS_DIR).join(format!("{id}.json")));
                return false;
            }
            if matches!(item.status, InboxStatus::Expired | InboxStatus::Withdrawn)
                && record
                    .stored
                    .terminal_at_ms
                    .is_some_and(|at| now.saturating_sub(at) > 30 * DAY_MS)
            {
                let _ = std::fs::remove_file(self.dir.join(PAGES_DIR).join(format!("{id}.html")));
                let _ = std::fs::remove_file(self.dir.join(ITEMS_DIR).join(format!("{id}.json")));
                return false;
            }
            true
        });
        let changed_items = changed;
        drop(inner);
        for (item, csp) in changed_items {
            let _ = self.write(&item, csp.as_deref());
        }
        self.bump();
    }
}
