use wonton_protocol::BotActivityFacts;

use super::bots::is_bot;
use super::store::StoreError;
use super::{Health, PluginId, PluginManager, PluginStatus};

impl PluginManager {
    pub(crate) fn bot_activity_facts(&self) -> Result<Vec<BotActivityFacts>, StoreError> {
        let plugins = self.plugins.read().expect("插件表锁中毒");
        let mut bots = Vec::new();
        for (id, entry) in plugins.iter() {
            if !entry
                .listed
                .as_ref()
                .is_some_and(|listed| is_bot(&listed.manifest))
            {
                continue;
            }
            let records = self.store.bots().load(id)?;
            let running =
                entry.status == PluginStatus::Running && entry.health == Some(Health::Healthy);
            for bot in records.bots.values() {
                bots.push(BotActivityFacts {
                    plugin_id: id.to_string(),
                    user_id: bot.account.user_id.clone(),
                    running,
                    model_requests: 0,
                    tool_calls: 0,
                    waiting: 0,
                    profile_rev: self.bot_profile_rev(id),
                });
            }
        }
        Ok(bots)
    }

    pub(crate) fn bot_profile_rev(&self, id: &PluginId) -> u64 {
        // 与资料写入共用 #528 的版本表（plugin/bot_profile.rs），核心重启后从零算。
        self.profile_revisions().get(id.as_str())
    }
}
