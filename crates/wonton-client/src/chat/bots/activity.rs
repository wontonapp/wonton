use tokio::time::Instant;
use wonton_protocol::BotChatAvailability;

use super::super::bot_activity::synthesize;
use super::super::lock;
use super::Bots;

impl Bots {
    pub fn clear_activity(&self) {
        let mut local = lock(&self.local);
        local.report = None;
        // 后台或换会话时网页状态作废：换一代，把已发的网页地址撤下来。
        local.web_generation = local.web_generation.wrapping_add(1);
        self.view.send_modify(|view| {
            for bot in &mut view.bots {
                bot.web = None;
                bot.web_page = None;
                // 没新的快照就核实不了资格：不再声称“能用”或“不能用”，壳照常能发、只写还没核实。
                bot.availability = BotChatAvailability::Unknown;
            }
        });
        for record in local.records.values_mut() {
            record.facts = None;
            record.typing.clear();
            record.activity = synthesize(&record.activity, None, false, Instant::now());
        }
        self.publish_activity(&local);
        self.changed.notify_one();
    }

    pub fn update_activity(&self) -> Option<Instant> {
        let mut local = lock(&self.local);
        let now = Instant::now();
        for record in local.records.values_mut() {
            record.activity = synthesize(
                &record.activity,
                record.facts.as_ref(),
                record.typing.values().any(|typing| *typing),
                now,
            );
        }
        self.publish_activity(&local);
        local
            .records
            .values()
            .filter_map(|record| record.activity.next_change)
            .min()
    }

    fn publish_activity(&self, local: &Local) {
        self.view.send_if_modified(|view| {
            let mut changed = false;
            for bot in &mut view.bots {
                let activity = local
                    .records
                    .get(&bot.bot)
                    .and_then(|record| record.activity.state);
                changed |= bot.activity != activity;
                bot.activity = activity;
            }
            changed
        });
    }
}
