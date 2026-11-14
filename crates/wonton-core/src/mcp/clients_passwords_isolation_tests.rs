//! Bot 和插件经 MCP 总线拿不到专用密码：核心工具里没有相关工具（ADR-0021「在 App 里」）。

use super::core_tools::definitions;

#[test]
fn no_mcp_tool_for_passwords() {
    for tool in definitions() {
        let rendered = serde_json::to_string(&tool).expect("工具定义可序列化");
        assert!(
            !rendered.contains("password"),
            "MCP 总线的工具不该提到专用密码：{}",
            tool.name
        );
    }
}
