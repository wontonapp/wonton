use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use super::{ConfirmationState, Confirmations, Table, now_ms};
use crate::bot_activity::BotActivity;

impl Confirmations {
    pub(crate) fn attach_activity(&self, activity: Arc<BotActivity>) {
        *self.activity.write().expect("确认表的活动表锁中毒") = Some(activity);
    }

    pub(super) fn activity_changed(&self) {
        if let Some(activity) = self.activity.read().expect("确认表的活动表锁中毒").clone()
        {
            activity.changed();
        }
    }

    fn expire_pending(&self, table: &mut Table, now: u64) {
        for slot in table.slots.values_mut() {
            if slot.waiter.is_some() && slot.card.expires_at_ms <= now {
                slot.waiter = None;
                slot.card.state = ConfirmationState::Expired;
                slot.ended = Some(now);
                self.activity_changed();
                self.bump();
            }
        }
    }

    pub(crate) fn pending_by_caller(&self) -> BTreeMap<String, u32> {
        let now = now_ms(SystemTime::now());
        let mut table = self.lock();
        self.expire_pending(&mut table, now);
        let mut counts = BTreeMap::new();
        for slot in table.slots.values().filter(|slot| slot.waiter.is_some()) {
            // card.plugin_id 是安装目标，不是发起确认的 Bot。
            *counts.entry(slot.caller.to_string()).or_default() += 1;
        }
        counts
    }

    pub(crate) fn next_pending_expiry(&self) -> Option<Duration> {
        let now = now_ms(SystemTime::now());
        self.lock()
            .slots
            .values()
            .filter(|slot| slot.waiter.is_some())
            .map(|slot| Duration::from_millis(slot.card.expires_at_ms.saturating_sub(now)))
            .min()
    }
}
