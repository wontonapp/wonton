//! 联系人系统需要的本机 Bot 信息（ADR-0045、ADR-0050）：用户 ID 反查、名字、名单重建。
//! 实现靠 [`crate::contacts::BotDirectory`] 挂到 [`Contacts`] 上，避免互相持有构造依赖。

use std::sync::Arc;

use wonton_protocol::BotRef;

use super::PluginManager;
use crate::contacts::{BotDirectory, Contacts};

impl PluginManager {
    /// 联系人行里存的插件 ID 还原成 [`PluginId`]；不是合法 ID（该是不会有）时为空。
    fn plugin_id(&self, raw: &str) -> Option<crate::plugin::PluginId> {
        crate::plugin::PluginId::parse(raw).ok()
    }

    /// 挂上联系人表：名单重建要问它。
    pub fn attach_contacts(self: &Arc<Self>, contacts: Arc<Contacts>) {
        let _ = self.contacts.set(contacts);
    }

    /// 归属人的显示名：家人在家人记录里，主人是主人资料里的。
    pub(super) fn keeper_display_name_of(&self, username: &str) -> Option<String> {
        if username == self.cfg.owner {
            return Some(self.cfg.profile.owner().display_name);
        }
        self.person_members
            .get()
            .and_then(|members| members.get(username))
            .filter(|record| record.status != wonton_protocol::MemberStatus::Removed)
            .map(|record| record.display_name)
    }
}

/// 实现在 `Arc<PluginManager>` 上：rebuild 是后台任务，要把句柄整个搬进去。
impl BotDirectory for Arc<PluginManager> {
    fn bot_of_user(&self, user_id: &str) -> Option<BotRef> {
        let all = self.store.bots().all().ok()?;
        for (id, bots) in all {
            for (bot, entry) in bots.bots {
                if entry.account.user_id == user_id {
                    return Some(BotRef {
                        plugin_id: id.to_string(),
                        bot,
                    });
                }
            }
        }
        None
    }

    fn user_of_bot(&self, bot: &BotRef) -> Option<String> {
        let id = self.plugin_id(&bot.plugin_id)?;
        self.store
            .bots()
            .load(&id)
            .ok()?
            .user_id(&bot.bot)
            .map(str::to_owned)
    }

    fn bot_localpart(&self, bot: &BotRef) -> Option<String> {
        let id = self.plugin_id(&bot.plugin_id)?;
        PluginManager::bot_localpart(self, &id, &bot.bot)
            .ok()
            .flatten()
    }

    fn display_name(&self, _bot: &BotRef) -> Option<String> {
        // Bot 的显示名在 Matrix 资料里（ADR-0044），发请求时现取存进联系人行；这里不给。
        None
    }

    fn keeper_display_name(&self, bot: &BotRef) -> Option<String> {
        let id = self.plugin_id(&bot.plugin_id)?;
        let entry = self
            .store
            .bots()
            .load(&id)
            .ok()?
            .bots
            .get(&bot.bot)?
            .clone();
        self.keeper_display_name_of(&self.keeper_user(&entry.keeper))
    }

    fn keeper_username(&self, bot: &BotRef) -> Option<String> {
        let id = self.plugin_id(&bot.plugin_id)?;
        let entry = self
            .store
            .bots()
            .load(&id)
            .ok()?
            .bots
            .get(&bot.bot)?
            .clone();
        match &entry.keeper {
            // 私人 Bot 的归属人是家人时才有单独的用户名；共享 Bot 归主人，推送走主人设备。
            wonton_protocol::BotKeeper::User { username } if *username != self.cfg.owner => {
                Some(username.clone())
            }
            _ => None,
        }
    }

    fn rebuild_bot(&self, bot: &BotRef) {
        let Some(id) = self.plugin_id(&bot.plugin_id) else {
            tracing::warn!(plugin = %bot.plugin_id, "联系人名单变化：插件 ID 认不出");
            return;
        };
        let manager = self.clone();
        let bot = bot.bot.clone();
        tokio::spawn(async move {
            // 同名的是 trait 方法（会递归回来）；这里要的是装容器的那一个。
            if let Err(e) = PluginManager::rebuild_bot(&manager, &id, &bot).await {
                // 容器还按旧名单跑：下次重建或核心重启会带上新名单。
                tracing::error!(plugin = %id, bot = %bot, error = %e, "按新联系人名单重建 Bot 失败");
            }
        });
    }
}
