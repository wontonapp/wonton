//! 调用方身份：Bot 的名字和引用。

use super::*;

impl Bus {
    /// 文件访问范围里的这个调用方：令牌认出的 Bot 用它的 Matrix 用户名（和 `grants` 查到的一样）；没有 Bot 的（比如
    /// 不进聊天的工具插件）用插件 ID，它有点，用户名里没有，不会重名。
    pub(super) fn bot_name(&self, caller: &Caller) -> Result<String, Failure> {
        let Some(bot) = &caller.bot else {
            return Ok(caller.id.to_string());
        };
        let localpart = self
            .plugins
            .bot_localpart(&caller.id, &bot.bot)
            .map_err(|e| Failure::new(FileFailure::Internal, e.to_string()))?;
        Ok(localpart.unwrap_or_else(|| caller.id.to_string()))
    }

    pub(super) fn bot_ref(&self, caller: &Caller, plugin: &PluginId) -> Result<BotRef, Failure> {
        Ok(BotRef {
            name: self.bot_name(caller)?,
            plugin: plugin.clone(),
        })
    }

    /// `tools/call` 转给插件时的调用者身份（ADR-0088 第 8 节）：私人 Bot 带归属人，主人和在这个
    /// 应用里当管理员（“能打开”且勾了“当管理员”）的人再带 [`Capability::PluginAdmin`]；共享 Bot
    /// 和认不出的人两个都不带，插件只能按全家共用的数据处理。认人的口径和工具放行一致（`bot_access`）：
    /// 记录在、人没被移除，就认。
    pub(crate) fn tool_call_identity(
        &self,
        caller: &Caller,
        plugin: &PluginId,
    ) -> upstream::CallIdentity {
        use wonton_protocol::{BotKeeper, Capability, MemberStatus};
        let Some(username) = caller.bot.as_ref().and_then(|bot| match &bot.keeper {
            BotKeeper::User { username } => Some(username),
            BotKeeper::Household | BotKeeper::Other => None,
        }) else {
            return upstream::CallIdentity::EMPTY;
        };
        if username == self.plugins.owner_name() {
            return upstream::CallIdentity {
                user: Some(username.clone()),
                capabilities: vec![Capability::PluginAdmin.as_str().to_owned()],
            };
        }
        let people = self.people.get();
        let Some(member) = people
            .and_then(|people| people.members.get(username))
            .filter(|member| member.status != MemberStatus::Removed)
        else {
            return upstream::CallIdentity::EMPTY;
        };
        let capabilities = people
            .and_then(|people| people.roles.get(&member.role_id))
            .filter(|role| {
                role.permissions.open_apps.allows_plugin(plugin.as_str())
                    && role.permissions.plugin_admin.allows_plugin(plugin.as_str())
            })
            .map(|_| vec![Capability::PluginAdmin.as_str().to_owned()])
            .unwrap_or_default();
        upstream::CallIdentity {
            user: Some(username.clone()),
            capabilities,
        }
    }
}
