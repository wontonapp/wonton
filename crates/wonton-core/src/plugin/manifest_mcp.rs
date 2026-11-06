//! 插件清单里的 MCP 服务声明与校验：总线连哪个服务、发请求带什么头、工具调用的期限、只提供哪些工具。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::manifest::ManifestError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum McpTransport {
    StreamableHttp,
}

/// 工具调用的期限最多声明这么长：一周。总线不能无限期地等一个调用。
const MAX_TOOL_SECONDS: u64 = 7 * 24 * 3600;
/// 不写 `tool_total_seconds` 时的总期限。
pub(super) const DEFAULT_TOOL_TOTAL_SECONDS: u64 = 30 * 60;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpEndpoint {
    pub service: String,
    pub port: u16,
    #[serde(default)]
    pub path: Option<String>,
    pub transport: McpTransport,
    /// 加在核心发往 MCP 上游的每个请求上的头：名字 → 值模板，见 [`super::proxy_auth`]。
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    /// 工具调用多少秒没有进度就算没有结果；不写时用默认值（[`super::ToolLimits`]）。
    #[serde(default)]
    pub tool_idle_seconds: Option<u64>,
    /// 工具调用最多多少秒；不写时用默认值。
    #[serde(default)]
    pub tool_total_seconds: Option<u64>,
    /// 只提供列表里的工具（上游的原名，允许列表）；不写时全部提供。
    #[serde(default)]
    pub tools: Option<Vec<String>>,
}

/// MCP 2025-11-25 规范（<https://modelcontextprotocol.io/specification/2025-11-25/server/tools> 的
/// "Tool Names"）对工具名的规定：SHOULD 级——1 到 128 个字符，只有 ASCII 字母、数字、`_`、`-` 和 `.`，
/// 大小写敏感。清单按它把关，写错的清单装不上。
fn is_tool_name(name: &str) -> bool {
    (1..=128).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
}

pub(super) fn validate(mcp: &McpEndpoint) -> Result<(), ManifestError> {
    if mcp
        .path
        .as_deref()
        .is_some_and(|path| !path.starts_with('/'))
    {
        return Err(ManifestError::Invalid("mcp.path 必须以 / 开头".to_owned()));
    }
    for (name, seconds) in [
        ("mcp.tool_idle_seconds", mcp.tool_idle_seconds),
        ("mcp.tool_total_seconds", mcp.tool_total_seconds),
    ] {
        if seconds.is_some_and(|seconds| seconds == 0 || seconds > MAX_TOOL_SECONDS) {
            return Err(ManifestError::Invalid(format!(
                "{name} 要在 1 到 {MAX_TOOL_SECONDS} 之间"
            )));
        }
    }
    if let Some(tools) = &mcp.tools {
        if tools.is_empty() {
            return Err(ManifestError::Invalid(
                "mcp.tools 至少写一个工具名；要全部提供就不写这个字段".to_owned(),
            ));
        }
        let mut seen = std::collections::BTreeSet::new();
        for name in tools {
            if !is_tool_name(name) {
                return Err(ManifestError::Invalid(format!(
                    "mcp.tools 里的 {name:?} 不是合法的工具名：1 到 128 个字符，只能有 ASCII 字母、数字、_、- 和 ."
                )));
            }
            if !seen.insert(name.as_str()) {
                return Err(ManifestError::Invalid(format!(
                    "mcp.tools 里的 {name} 重复了"
                )));
            }
        }
    }
    // 总期限先到，空闲期限就白写了：长工具每次到总期限都被截断。
    let total = mcp.tool_total_seconds.unwrap_or(DEFAULT_TOOL_TOTAL_SECONDS);
    if mcp.tool_idle_seconds.is_some_and(|idle| idle > total) {
        return Err(ManifestError::Invalid(format!(
            "mcp.tool_idle_seconds 不能超过总期限（{total} 秒），把 mcp.tool_total_seconds 也写上"
        )));
    }
    Ok(())
}
