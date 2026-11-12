//! 放宽确认的设置：工具列表和各个开关（ADR-0024）。一次调用怎么判定见 [`Bus::call_plugin`]。

use std::collections::BTreeSet;
use std::time::Duration;

use wonton_protocol::{
    AddedTools, BotSwitch, OpenWindow, ToolAccess, ToolPolicyEntry, ToolPolicyView,
};

use super::Bus;
use super::audit::PolicyRecord;
use super::cards;
use crate::plugin::PluginId;
use crate::plugin::tool_policy::{PolicyUnavailable, ToolPolicy};

const NEW_VERSION_TOOL_RETRIES: u32 = 15;
const NEW_VERSION_TOOL_WAIT: Duration = Duration::from_secs(1);

/// 主人要改的一个开关。
#[derive(Debug, Clone, Copy)]
pub enum PolicyChange<'a> {
    AllowAll { caller: &'a str },
    Always { plugin: &'a str, caller: &'a str },
    ReadOnly { plugin: &'a str, tool: &'a str },
}

impl<'a> PolicyChange<'a> {
    fn apply(self, policy: &ToolPolicy, enabled: bool) -> Result<(), PolicyUnavailable> {
        match self {
            Self::AllowAll { caller } => policy.set_allow_all(caller, enabled),
            Self::Always { plugin, caller } => policy.set_always(plugin, caller, enabled),
            Self::ReadOnly { plugin, tool } => policy.set_read_only(plugin, tool, enabled),
        }
    }

    fn audited(self, enabled: bool, by: &'a str) -> PolicyRecord<'a> {
        let (policy, plugin, caller, tool) = match self {
            Self::AllowAll { caller } => ("allow_all", None, Some(caller), None),
            Self::Always { plugin, caller } => ("always", Some(plugin), Some(caller), None),
            Self::ReadOnly { plugin, tool } => ("read_only", Some(plugin), None, Some(tool)),
        };
        PolicyRecord {
            policy,
            plugin,
            caller,
            tool,
            enabled,
            by,
        }
    }
}

impl Bus {
    pub async fn tool_policy_view(&self, plugin: &PluginId) -> ToolPolicyView {
        let policy = self.plugins.tool_policy();
        let callers = self.plugins.tool_callers();
        let name = plugin.as_str();
        // 没有工具的插件，始终允许不起作用。
        let declares_mcp = self.plugins.declares_mcp(plugin);
        let always = callers
            .iter()
            .filter(|_| declares_mcp)
            .map(|(caller, title)| BotSwitch {
                caller: caller.to_string(),
                name: cards::text(title),
                enabled: policy.is_always(name, caller.as_str()),
            });
        let allow_all =
            callers
                .iter()
                .filter(|(caller, _)| caller == plugin)
                .map(|(caller, title)| BotSwitch {
                    caller: caller.to_string(),
                    name: cards::text(title),
                    enabled: policy.is_allow_all(caller.as_str()),
                });
        let windows = self.confirmations.active_windows(name).into_iter();
        let windows = windows.map(|window| OpenWindow {
            name: callers
                .iter()
                .find(|(caller, _)| caller == &window.caller)
                .map_or_else(
                    || window.caller.to_string(),
                    |(_, title)| cards::text(title),
                ),
            caller: window.caller.to_string(),
            card: window.card,
            expires_at_ms: window.expires_at_ms,
        });
        let windows = windows.collect();
        let (tools, tools_error) = match self.listed_tools(plugin).await {
            Ok(tools) => (tools, None),
            Err(e) => (Vec::new(), Some(e)),
        };
        ToolPolicyView {
            tools,
            tools_error,
            always: always.collect(),
            allow_all: allow_all.collect(),
            windows,
            problem: policy.problem(),
        }
    }

    async fn listed_tools(&self, plugin: &PluginId) -> Result<Vec<ToolPolicyEntry>, String> {
        if !self.plugins.declares_mcp(plugin) {
            return Ok(Vec::new());
        }
        let servers = self.plugins.mcp_servers();
        let Some(server) = servers.into_iter().find(|server| &server.id == plugin) else {
            return Err("插件没有运行，列不出工具".to_owned());
        };
        let policy = self.plugins.tool_policy();
        let tools = self.upstreams.tools(&server).await?;
        let entry = |tool: rmcp::model::Tool| {
            let read_only = tool.annotations.as_ref().and_then(|a| a.read_only_hint);
            let access = if read_only == Some(true) {
                ToolAccess::PluginReadOnly
            } else if policy.is_read_only(plugin.as_str(), &tool.name) {
                ToolAccess::OwnerReadOnly
            } else {
                ToolAccess::Sensitive
            };
            ToolPolicyEntry {
                name: tool.name.to_string(),
                description: tool.description.map(|text| text.to_string()),
                access,
            }
        };
        Ok(tools.into_iter().map(entry).collect())
    }

    /// 升级前记下工具名，给 [`Self::added_tools`] 比较。
    pub async fn tool_names(&self, plugin: &PluginId) -> Result<BTreeSet<String>, String> {
        let tools = self.listed_tools(plugin).await?;
        Ok(tools.into_iter().map(|entry| entry.name).collect())
    }

    /// 新版本比 `before` 多了哪些工具。新版本的健康检查过了不代表 MCP 服务起来了，所以列不出来时多等一会儿。
    pub async fn added_tools(
        &self,
        plugin: &PluginId,
        before: Result<BTreeSet<String>, String>,
    ) -> AddedTools {
        let before = match before {
            Ok(before) => before,
            Err(reason) => {
                return AddedTools::NotCompared {
                    reason: format!("旧版本列不出工具：{reason}"),
                };
            }
        };
        let mut after = self.tool_names(plugin).await;
        for _ in 0..NEW_VERSION_TOOL_RETRIES {
            if after.is_ok() {
                break;
            }
            tokio::time::sleep(NEW_VERSION_TOOL_WAIT).await;
            after = self.tool_names(plugin).await;
        }
        match after {
            Ok(after) => AddedTools::Compared {
                names: after.difference(&before).cloned().collect(),
            },
            Err(reason) => AddedTools::NotCompared {
                reason: format!("新版本列不出工具：{reason}"),
            },
        }
    }

    /// 插件列得出工具，里面却没有这一个。列不出来时不算：插件没运行时主人也能取消标记。
    pub async fn lacks_tool(&self, plugin: &PluginId, tool: &str) -> bool {
        self.listed_tools(plugin)
            .await
            .is_ok_and(|tools| !tools.iter().any(|entry| entry.name == tool))
    }

    /// 改一个开关，写一行审计。`by` 是谁改的。
    pub fn change_tool_policy(
        &self,
        change: PolicyChange<'_>,
        enabled: bool,
        by: &str,
    ) -> Result<(), PolicyUnavailable> {
        change.apply(self.plugins.tool_policy(), enabled)?;
        self.audit.record(&change.audited(enabled, by));
        Ok(())
    }
}
