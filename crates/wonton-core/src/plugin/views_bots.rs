//! 插件视图里按 Bot 附带的字段：Bot 用户名、写它的人；读记录要读文件，在插件表锁外做。

use std::collections::BTreeSet;

use wonton_protocol::InstalledPlugin;

use super::manager::PluginManager;
use super::manifest::PluginId;
use super::store::StoreError;

impl PluginManager {
    /// 全部 Bot 用掉的 Matrix 用户名（localpart），给家人用户名的查重用（ADR-0045）。
    pub fn bot_localparts(&self) -> Result<BTreeSet<String>, StoreError> {
        self.store.bots().localparts()
    }

    /// 填上插件的 Bot 和写它的人：读记录要读文件，不在插件表的锁里做。
    pub(super) fn with_bot(&self, mut plugin: InstalledPlugin) -> InstalledPlugin {
        let Ok(id) = PluginId::parse(&plugin.id) else {
            return plugin;
        };
        match self.store.bots().load(&id) {
            Ok(record) => {
                plugin.bot_user_id = record
                    .user_id(super::bot_services::DEFAULT_BOT)
                    .map(str::to_owned)
            }
            Err(e) => tracing::warn!(plugin = %id, error = %e, "读 Bot 记录失败"),
        }
        match self.store.load(&id) {
            Ok(record) => {
                plugin.written_by = record.as_ref().and_then(|record| record.written_by.clone());
                plugin.enabled_root_permissions = record
                    .as_ref()
                    .map(|record| {
                        record
                            .enabled_permissions
                            .iter()
                            .filter(|item| {
                                super::privileges::risk(&item.permission)
                                    == Some(super::privileges::CapabilityRisk::Highest)
                            })
                            .map(|item| item.permission.to_string())
                            .collect()
                    })
                    .unwrap_or_default();
                plugin.family = record.is_some_and(|record| record.family);
            }
            Err(e) => tracing::warn!(plugin = %id, error = %e, "读插件记录失败"),
        }
        plugin
    }
}

#[cfg(test)]
impl PluginManager {
    pub(crate) fn bot_for_test(&self, id: &PluginId, user_id: &str) {
        self.store
            .bots()
            .save(
                id,
                &super::bots::PluginBots {
                    bots: [(
                        super::bot_services::DEFAULT_BOT.to_owned(),
                        super::bots::BotEntry {
                            account: crate::matrix::BotAccount {
                                user_id: user_id.to_owned(),
                                home_room: "!test:example.com".to_owned(),
                            },
                            device_id: None,
                            keeper: wonton_protocol::BotKeeper::User {
                                username: "owner".to_owned(),
                            },
                            bus_token: "test-token".to_owned(),
                        },
                    )]
                    .into_iter()
                    .collect(),
                },
            )
            .unwrap();
    }
}
