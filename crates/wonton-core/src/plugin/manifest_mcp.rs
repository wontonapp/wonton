//! 插件清单里的 MCP 服务声明与校验：总线连哪个服务、发请求带什么头、工具调用的期限、只提供哪些工具。

use super::manifest::ManifestError;

pub use wonton_protocol::manifest::{McpEndpoint, McpTransport};

/// 工具调用的期限最多声明这么长：一周。总线不能无限期地等一个调用。
const MAX_TOOL_SECONDS: u64 = 7 * 24 * 3600;
/// 不写 `tool_total_seconds` 时的总期限。
pub(super) const DEFAULT_TOOL_TOTAL_SECONDS: u64 = 30 * 60;

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
        return Err(ManifestError::Invalid(
            "mcp.path must start with /".to_owned(),
        ));
    }
    validate_limits(
        "mcp",
        mcp.tool_idle_seconds,
        mcp.tool_total_seconds,
        mcp.tools.as_deref(),
    )
}

/// 工具白名单和两个期限，`prefix` 是写进报错的字段前缀（`mcp`、`remote_mcp`）。
pub(super) fn validate_limits(
    prefix: &str,
    idle: Option<u64>,
    total_seconds: Option<u64>,
    tools: Option<&[String]>,
) -> Result<(), ManifestError> {
    for (name, seconds) in [
        (format!("{prefix}.tool_idle_seconds"), idle),
        (format!("{prefix}.tool_total_seconds"), total_seconds),
    ] {
        if seconds.is_some_and(|seconds| seconds == 0 || seconds > MAX_TOOL_SECONDS) {
            return Err(ManifestError::Invalid(format!(
                "{name} must be between 1 and {MAX_TOOL_SECONDS}"
            )));
        }
    }
    if let Some(tools) = tools {
        if tools.is_empty() {
            return Err(ManifestError::Invalid(format!(
                "{prefix}.tools needs at least one tool name; omit the field to expose all tools"
            )));
        }
        let mut seen = std::collections::BTreeSet::new();
        for name in tools {
            if !is_tool_name(name) {
                return Err(ManifestError::Invalid(format!(
                    "{prefix}.tools entry {name:?} is not a valid tool name: 1 to 128 characters, only ASCII letters, digits, _, -, and ."
                )));
            }
            if !seen.insert(name.as_str()) {
                return Err(ManifestError::Invalid(format!(
                    "{prefix}.tools entry {name} is duplicated"
                )));
            }
        }
    }
    // 总期限先到，空闲期限就白写了：长工具每次到总期限都被截断。
    let total = total_seconds.unwrap_or(DEFAULT_TOOL_TOTAL_SECONDS);
    if idle.is_some_and(|idle| idle > total) {
        return Err(ManifestError::Invalid(format!(
            "{prefix}.tool_idle_seconds must not exceed the total timeout ({total} s); set {prefix}.tool_total_seconds as well"
        )));
    }
    Ok(())
}
