//! 总线要连的 MCP 服务器和工具调用的期限。

use std::path::PathBuf;
use std::time::Duration;

use super::manifest::{DEFAULT_TOOL_TOTAL_SECONDS, LocalizedText, McpEndpoint, PluginId};
use super::proxy_auth::McpHeaders;

/// 总线连接的插件端点：容器的服务，或远端 MCP 的连接器。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpServer {
    pub id: PluginId,
    pub title: LocalizedText,
    /// `http://主机:端口/路径`；宿主网络里主机名是占位的 `localhost`。
    pub url: String,
    /// 宿主网络里经 unix 套接字连。
    pub socket: Option<PathBuf>,
    /// 发往端点的凭据与配置头；`Debug` 不打印值。
    pub headers: McpHeaders,
    pub tool_limits: ToolLimits,
    /// 清单只提供这些工具（上游的原名，允许列表）；不写时全部提供。
    pub tools: Option<Vec<String>>,
}

/// 工具调用等多久（插件规范的“工具调用的期限”）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolLimits {
    /// 多久没有进度就不等了。
    pub idle: Duration,
    /// 不管有没有进度，最多等多久。
    pub total: Duration,
}

impl Default for ToolLimits {
    fn default() -> Self {
        Self {
            idle: Duration::from_secs(10 * 60),
            total: Duration::from_secs(DEFAULT_TOOL_TOTAL_SECONDS),
        }
    }
}

impl ToolLimits {
    pub(super) fn of(mcp: Option<&McpEndpoint>) -> Self {
        let default = Self::default();
        let declared = |seconds: fn(&McpEndpoint) -> Option<u64>, or| {
            mcp.and_then(seconds).map_or(or, Duration::from_secs)
        };
        Self {
            idle: declared(|mcp| mcp.tool_idle_seconds, default.idle),
            total: declared(|mcp| mcp.tool_total_seconds, default.total),
        }
    }
}
