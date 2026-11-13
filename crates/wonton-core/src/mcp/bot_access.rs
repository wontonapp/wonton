//! 插件工具同时受归属人的 `open_apps`（ADR-0088）和单 Bot 开关（ADR-0066）限制。
//! 共享 Bot 按主人；没有 Bot 身份的工具插件不走这层归属人授权。

use wonton_protocol::{BotKeeper, MemberStatus};

use super::server::Caller;
use super::{Bus, ToolFailure};
use crate::plugin::PluginId;

impl Bus {
    /// `caller` 能不能用 `target` 的工具；不能时是给 Agent 的失败。
    pub(super) fn bot_access(&self, caller: &Caller, target: &PluginId) -> Result<(), ToolFailure> {
        let Some(bot) = &caller.bot else {
            return Ok(());
        };
        let allowed = match &bot.keeper {
            BotKeeper::User { username } if username == self.plugins.owner_name() => true,
            BotKeeper::User { username } => self.people.get().is_some_and(|people| {
                people.members.get(username).is_some_and(|member| {
                    member.status != MemberStatus::Removed
                        && people.roles.get(&member.role_id).is_some_and(|role| {
                            role.permissions.open_apps.allows_plugin(target.as_str())
                        })
                })
            }),
            BotKeeper::Household => true,
            BotKeeper::Other => false,
        };
        if !allowed {
            return Err(ToolFailure::new(
                "permission_denied",
                format!("这个 Bot 的归属人不能打开应用 {target}"),
            ));
        }
        let policy = self.plugins.tool_policy();
        match policy.is_bot_off(caller.id.as_str(), &bot.bot, target.as_str()) {
            Ok(false) => Ok(()),
            Ok(true) => Err(ToolFailure::new(
                "permission_denied",
                format!("主人没让这个 Bot 用插件 {target} 的工具"),
            )),
            // 不知道主人关了哪些，不当作都开着。
            Err(e) => Err(ToolFailure::new("policy_unavailable", e.to_string())),
        }
    }
}
