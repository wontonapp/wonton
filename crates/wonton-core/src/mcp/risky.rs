//! 插件工具按 MCP 注解分级（ADR-0088 第 5 节），和每次都要主人确认的三类操作（ADR-0024 的“每次都问的三类”：
//! 删东西、装应用、开放到外面）。装应用和开放到外面只有核心自己的工具做得到（`wonton__install_plugin`、
//! `wonton__set_plugin_permission` 打开），它们本来就每次发卡片；这里认出插件工具里删东西的那一类。

use rmcp::model::Tool;
use serde_json::Value;
use wonton_protocol::AiLevel;

use super::audience::Subject;
use super::cards::{self, Draft};
use super::{Bus, Caller};
use crate::plugin::{McpServer, PluginId};

/// 工具按注解映射到三种 AI 动手级别（ADR-0088 第 5 节）：只读、会改不删、会删会覆盖。
/// 这里的级别是工具类别，不是调用方封顶：`AdminApprovesDeletes` 和 `SelfConfirms` 都能走确认流程，
/// 但前者把会删的卡片交给管理员，后者交给归属人。
pub(super) fn classify(tool: &Tool) -> AiLevel {
    let Some(annotations) = &tool.annotations else {
        return AiLevel::SelfConfirms;
    };
    if annotations.read_only_hint == Some(true) {
        AiLevel::ReadOnly
    } else if annotations.destructive_hint == Some(false) {
        AiLevel::AdminApprovesDeletes
    } else {
        AiLevel::SelfConfirms
    }
}

/// 封顶 [`AiLevel::ReadOnly`] 的人只有只读的工具，另外两档都拿不到。
pub(super) fn capped(caller_level: AiLevel, tool_level: AiLevel) -> bool {
    caller_level == AiLevel::ReadOnly && tool_level != AiLevel::ReadOnly
}

/// 插件明确标了 `destructiveHint: true`，又没标只读。MCP 规范里不标时默认算真，但那样不标注解的社区工具全算进来，
/// 放宽就形同虚设（ADR-0024），所以只认明确标了的。规范说只读时这个注解没有意义，只读照旧不问。
pub(super) fn deletes(tool: &Tool) -> bool {
    tool.annotations
        .as_ref()
        .is_some_and(|a| a.read_only_hint != Some(true) && a.destructive_hint == Some(true))
}

/// 放行时段管谁（ADR-0024 的“这个 Bot 调这个插件”）：`Caller::id` 对同一个插件的所有 Bot 都一样，
/// 时段按 Bot 分开，家人放宽自己的 Bot 时不连主人的 Bot 一起放行。
pub(super) fn window_scope(caller: &Caller, plugin: &str) -> String {
    match &caller.bot {
        Some(bot) => format!("{plugin}#{}", bot.bot),
        None => plugin.to_owned(),
    }
}

/// 工具调用在确认阶段使用的封顶决定。
#[derive(Clone, Copy)]
pub(super) struct ToolDecision {
    pub(super) caller_level: AiLevel,
    pub(super) highest: bool,
    pub(super) subject: Subject,
    pub(super) every_time: bool,
    pub(super) read_only: bool,
    pub(super) relaxed: bool,
    deletes: bool,
}

impl ToolDecision {
    pub(super) fn new(
        definition: &Tool,
        caller_level: AiLevel,
        highest: bool,
        subject: Subject,
        relaxed: bool,
    ) -> Self {
        let read_only = definition
            .annotations
            .as_ref()
            .and_then(|a| a.read_only_hint)
            == Some(true);
        let deletes = deletes(definition);
        Self {
            caller_level,
            highest,
            subject,
            every_time: highest || deletes,
            read_only,
            relaxed,
            deletes,
        }
    }
}

impl Bus {
    /// 插件工具的确认卡片。每次都问的工具（最高级插件的、明确标了会删的）没有“N 分钟内都允许”：时段放行不了它们，
    /// 按钮在上面就是骗主人。只有封顶是“改动自己确认”的调用能放宽（ADR-0088 第 5 节）。
    pub(super) fn tool_card(
        &self,
        caller: &Caller,
        server: &McpServer,
        definition: &Tool,
        args: &Value,
        decision: ToolDecision,
    ) -> Draft {
        let langs = self.card_languages(caller, decision.subject);
        let draft = cards::tool_call(
            langs.pick(&server.title),
            definition,
            args,
            decision.highest,
            &langs,
        );
        let relaxable = !decision.highest
            && !decision.deletes
            && decision.caller_level == AiLevel::SelfConfirms
            && super::handoff::allows_relaxation(self, caller);
        if !relaxable {
            return draft;
        }
        draft.with_window(
            &window_scope(caller, server.id.as_str()),
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
