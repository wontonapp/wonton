//! 读确认表：列表、单张卡片、列表版本和谁能回答。

use std::time::{Duration, SystemTime};

use wonton_protocol::ConfirmationCard;

use tokio::sync::watch;

use super::{Confirmations, Recipients, Slot, now_ms};

impl Confirmations {
    /// “等你确认”列表（ADR-0016）：等回答的（最早到期的在前）、结束不到保留期的（最近结束的在前），和当前版本。
    /// 只列发给这个人的（`None` 是主人）：家人的列表里没有主人和别人的卡片。
    pub fn list(
        &self,
        recipient: Option<&str>,
    ) -> (Vec<ConfirmationCard>, Vec<ConfirmationCard>, u64) {
        let now = now_ms(SystemTime::now());
        let table = self.lock();
        let mut pending: Vec<&Slot> = Vec::new();
        let mut finished: Vec<&Slot> = Vec::new();
        for slot in table.slots.values() {
            if !slot.includes(recipient) {
                continue;
            }
            if slot.waiter.is_some() && slot.card.expires_at_ms > now {
                pending.push(slot);
            } else if let Some(ended) = slot.ended
                && Duration::from_millis(now.saturating_sub(ended)) < self.keep
            {
                finished.push(slot);
            }
        }
        pending.sort_by_key(|slot| slot.card.expires_at_ms);
        finished.sort_by_key(|slot| std::cmp::Reverse(slot.ended));
        let revision = *self.revision.borrow();
        (
            pending.into_iter().map(|s| s.card.clone()).collect(),
            finished.into_iter().map(|s| s.card.clone()).collect(),
            revision,
        )
    }

    /// 一张卡片：等回答的、和结束不到保留期的都取得到，再早的没有。收件人用 [`Self::recipient`] 查。
    pub fn card(&self, id: &str) -> Option<ConfirmationCard> {
        let now = now_ms(SystemTime::now());
        self.lock().slots.get(id).and_then(|slot| {
            let pending = slot.waiter.is_some() && slot.card.expires_at_ms > now;
            let kept = slot
                .ended
                .is_some_and(|ended| Duration::from_millis(now.saturating_sub(ended)) < self.keep);
            (pending || kept).then(|| slot.card.clone())
        })
    }

    /// 当前列表的版本。
    pub fn revision(&self) -> u64 {
        *self.revision.borrow()
    }

    /// 版本变化的订阅：网关的长轮询挂在它上面等下一张卡片。
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.revision.subscribe()
    }

    /// 单收件人卡片返回收件人；管理员卡片和不存在的 ID 返回 `None`。
    pub fn recipient(&self, id: &str) -> Option<Option<String>> {
        self.lock().slots.get(id).and_then(|slot| {
            matches!(&slot.recipients, Recipients::Keeper(_)).then(|| slot.recipients.keeper())
        })
    }

    /// 判断这个人是否属于卡片创建时冻结的收件人集合；不存在的 ID 返回 `None`。
    pub fn may_answer(&self, id: &str, recipient: Option<&str>) -> Option<bool> {
        self.lock()
            .slots
            .get(id)
            .map(|slot| slot.includes(recipient))
    }

    /// 回答接口的权限判断：管理员卡片已有其他人回答时，其他管理员不能重复回答。
    pub fn may_answer_as(
        &self,
        id: &str,
        recipient: Option<&str>,
        user_id: Option<&str>,
    ) -> Option<bool> {
        self.lock().slots.get(id).map(|slot| {
            slot.includes(recipient)
                && slot
                    .card
                    .answered_by
                    .as_deref()
                    .is_none_or(|answered_by| user_id == Some(answered_by))
        })
    }
}
