//! 按 Bot 放行插件的工具（ADR-0066 第 5 节）：主人对某个 Bot 单独关掉的插件，这个 Bot 在 `tools/list` 里看不到，
//! `tools/call` 回 `permission_denied`。没有 Bot 记录的调用方（按来源地址认的工具插件）不受影响。
//!
//! 插件范围（全局或指定的 Bot）只有“全局”一种在生效，所以没单独关掉的 Bot 都能用。

use super::server::Caller;
use super::{Bus, ToolFailure};
use crate::plugin::PluginId;

impl Bus {
    /// `caller` 能不能用 `target` 的工具；不能时是给 Agent 的失败。
    pub(super) fn bot_access(&self, caller: &Caller, target: &PluginId) -> Result<(), ToolFailure> {
        let Some(bot) = &caller.bot else {
            return Ok(());
        };
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
