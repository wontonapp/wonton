//! 每次都要主人确认的三类操作（ADR-0024 的“每次都问的三类”）：删东西、装应用、开放到外面。装应用和开放到外面只有
//! 核心自己的工具做得到（`wonton__install_plugin`、`wonton__set_plugin_permission` 打开），它们本来就每次发卡片；
//! 这里认出插件工具里删东西的那一类。

use rmcp::model::Tool;
use serde_json::Value;

use super::audience::Subject;
use super::cards::{self, Draft};
use super::{Bus, Caller};
use crate::l10n;
use crate::plugin::{McpServer, PluginId};

/// 插件明确标了 `destructiveHint: true`，又没标只读。MCP 规范里不标时默认算真，但那样不标注解的社区工具全算进来，
/// 放宽就形同虚设（ADR-0024），所以只认明确标了的。规范说只读时这个注解没有意义，只读照旧不问。
pub(super) fn deletes(tool: &Tool) -> bool {
    tool.annotations
        .as_ref()
        .is_some_and(|a| a.read_only_hint != Some(true) && a.destructive_hint == Some(true))
}

impl Bus {
    /// 插件工具的确认卡片。每次都问的工具（最高级插件的、会删东西的）没有“N 分钟内都允许”：时段放行不了它们，
    /// 按钮在上面就是骗主人。
    pub(super) fn tool_card(
        &self,
        caller: &Caller,
        server: &McpServer,
        definition: &Tool,
        args: &Value,
        highest: bool,
    ) -> Draft {
        let plugin = cards::text(&server.title);
        let langs = self.card_languages(caller, Subject::Tool);
        let mut draft = cards::tool_call(
            &caller.name,
            &plugin,
            &definition.name,
            args,
            highest,
            &langs,
        );
        let deletes = deletes(definition);
        if deletes && let Some(first) = draft.lines.first_mut() {
            *first = cards::emphasized(l10n::text(&langs, "chat-card-tool-deletes", &[]));
        }
        // 放行时段按插件记，不分 Bot：家人打开的时段会连主人的 Bot 一起放行，所以发给家人的卡片不给这个选项。
        if highest
            || deletes
            || self.recipient(caller, Subject::Tool).is_some()
            || !super::handoff::allows_relaxation(self, caller)
        {
            return draft;
        }
        draft.with_window(
            server.id.as_str(),
            self.confirmations.window_length(),
            &langs,
        )
    }

    /// 插件列得出工具，这一个会删东西。主人标的只读对它不生效，设置接口不收这样的标记。
    pub async fn tool_deletes(&self, plugin: &PluginId, tool: &str) -> bool {
        let servers = self.plugins.mcp_servers();
        let Some(server) = servers.into_iter().find(|server| &server.id == plugin) else {
            return false;
        };
        self.upstreams
            .tools(&server)
            .await
            .is_ok_and(|tools| tools.iter().any(|t| t.name == tool && deletes(t)))
    }
}
