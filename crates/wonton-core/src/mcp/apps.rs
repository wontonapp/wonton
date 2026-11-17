//! MCP Apps（ADR-0052 的“Bot 往面板上放页面”）：界面只经宿主页说话，工具调用和资源读取由核心转发。
//!
//! 界面能调的只限提供它的那个插件上、`_meta.ui.visibility` 含 `app` 的工具；调用不带人也不带能力，
//! 插件只能当全家共用的数据处理，敏感的照常请人确认（ADR-0016 的卡片）。这里只放转发本身，HTTP 入口在网关。

use std::sync::Arc;

use rmcp::model::{CallToolResult, JsonObject, Tool};
use serde_json::Value;

use super::{Bus, Caller, ToolFailure, naming, upstream::ReadError};
use crate::l10n;
use crate::plugin::{McpServer, PluginId};

/// 工具声明了给界面用（`_meta.ui.visibility` 含 `app`）。
pub(super) fn visible(tool: &Tool) -> bool {
    tool.meta
        .as_ref()
        .and_then(|meta| meta.0.get("ui"))
        .and_then(|ui| ui.get("visibility"))
        .and_then(Value::as_array)
        .is_some_and(|items| items.iter().any(|item| item == "app"))
}

fn unavailable(plugin: &PluginId) -> ToolFailure {
    ToolFailure::new(
        "source_unavailable",
        format!("插件 {plugin} 没有在运行的 MCP 服务"),
    )
}

impl Bus {
    fn app_server(&self, plugin: &PluginId) -> Result<McpServer, ToolFailure> {
        self.plugins
            .mcp_servers()
            .into_iter()
            .find(|server| server.id == *plugin)
            .ok_or_else(|| unavailable(plugin))
    }

    /// 界面能调的、这个插件声明的工具。
    pub(crate) async fn app_tools(&self, plugin: &PluginId) -> Result<Vec<Tool>, ToolFailure> {
        let server = self.app_server(plugin)?;
        let tools = self
            .upstreams
            .tools(&server)
            .await
            .map_err(|e| ToolFailure::new("source_unavailable", e))?;
        Ok(tools.into_iter().filter(visible).collect())
    }

    /// 界面发起的工具调用。返回的还是插件的原样结果（错误结果也原样）。
    pub(crate) async fn app_call(
        self: &Arc<Self>,
        plugin: &PluginId,
        tool: &str,
        arguments: JsonObject,
    ) -> Result<CallToolResult, ToolFailure> {
        let server = self.app_server(plugin)?;
        let definition = self
            .upstreams
            .tools(&server)
            .await
            .map_err(|e| ToolFailure::new("source_unavailable", e))?
            .into_iter()
            .find(|definition| definition.name == tool)
            .ok_or_else(|| {
                ToolFailure::new("not_found", format!("插件 {plugin} 没有工具 {tool}"))
            })?;
        if !visible(&definition) {
            return Err(ToolFailure::new(
                "not_found",
                format!("工具 {tool} 没有声明给界面用"),
            ));
        }
        let mut caller = Caller::new(server.id.clone(), Some(&server.title));
        caller.name = l10n::text(&self.owner_languages(), "mcp-app-owner", &[]);
        let name = naming::join(&server.id.slug(), tool);
        let activity = self.plugins.activity.start_tool(&server.id);
        Ok(self
            .call_plugin(
                &caller,
                &server,
                (&name, tool, activity),
                arguments,
                std::future::pending(),
            )
            .await)
    }

    /// 界面读同一个插件的资源。
    pub(crate) async fn app_read(
        &self,
        plugin: &PluginId,
        uri: &str,
    ) -> Result<rmcp::model::ReadResourceResult, ToolFailure> {
        let server = self.app_server(plugin)?;
        self.upstreams
            .read_contents(&server, uri)
            .await
            .map_err(|error| match error {
                ReadError::Unreachable(e) | ReadError::Upstream(e) => {
                    ToolFailure::new("source_unavailable", e)
                }
                ReadError::TooLarge => ToolFailure::new("too_large", "资源超过上限"),
            })
    }
}
