use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum McpTransport {
    StreamableHttp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpEndpoint {
    pub service: String,
    pub port: u16,
    #[serde(default)]
    pub path: Option<String>,
    pub transport: McpTransport,
    /// 加在核心发往 MCP 上游的每个请求上的头：名字 → 值模板，见 `webui.proxy.headers`。
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    /// 工具调用多少秒没有进度就算没有结果；不写时用默认值。
    #[serde(default)]
    pub tool_idle_seconds: Option<u64>,
    /// 工具调用最多多少秒；不写时用默认值。
    #[serde(default)]
    pub tool_total_seconds: Option<u64>,
    /// 只提供列表里的工具（上游的原名，允许列表）；不写时全部提供。
    #[serde(default)]
    pub tools: Option<Vec<String>>,
}
