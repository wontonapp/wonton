//! 清单里工具相关的声明：总期限、空闲期限和 `mcp.tools` 白名单的校验。

use super::*;

#[test]
fn a_plugin_can_declare_longer_tool_limits_within_reason() {
    let parse = |extra: &str| {
        let text = NOTES.replace("streamable-http}", &format!("streamable-http{extra}}}"));
        PluginPackage::parse(&text, &Languages::from_header(Some("zh-Hans")))
            .map(|package| package.manifest.mcp.unwrap())
    };
    let mcp = parse(", tool_idle_seconds: 3600, tool_total_seconds: 7200").unwrap();
    assert_eq!(mcp.tool_idle_seconds, Some(3600));
    assert_eq!(mcp.tool_total_seconds, Some(7200));
    assert!(parse(", tool_idle_seconds: 0").is_err());
    assert!(parse(", tool_total_seconds: 99999999").is_err());
    // 总期限先到，空闲期限就白写了。不写总期限时按默认的 30 分钟算。
    assert!(parse(", tool_idle_seconds: 7200, tool_total_seconds: 3600").is_err());
    assert!(parse(", tool_idle_seconds: 7200").is_err());
    assert!(parse(", tool_idle_seconds: 1800").is_ok());
    assert!(parse(", tool_idle_seconds: 3600, tool_total_seconds: 3600").is_ok());
}

/// 清单的 `mcp.tools`：不写时全部提供；写了就必须是非空、不重复的合法工具名列表（MCP 2025-11-25 的
/// 工具名规定，见 `manifest_mcp.rs`）。
#[test]
fn a_plugin_can_limit_its_tools_to_a_valid_allowlist() {
    let parse = |extra: &str| {
        let text = NOTES.replace("streamable-http}", &format!("streamable-http{extra}}}"));
        PluginPackage::parse(&text, &Languages::from_header(Some("zh-Hans")))
            .map(|package| package.manifest.mcp.unwrap())
    };
    let mcp = parse(", tools: [search, find, a.b_c-d]").unwrap();
    assert_eq!(
        mcp.tools,
        Some(vec!["search".into(), "find".into(), "a.b_c-d".into()])
    );
    // 不写这个字段就是全部提供，和现在一样。
    assert_eq!(parse("").unwrap().tools, None);
    for (extra, why) in [
        (", tools: []", "mcp.tools"),
        (", tools: [search, search]", "duplicated"),
        (", tools: [\"search files\"]", "not a valid tool name"),
        (", tools: [搜索]", "not a valid tool name"),
    ] {
        let error = parse(extra).unwrap_err().to_string();
        assert!(error.contains(why), "{extra}: {error}");
    }
}
