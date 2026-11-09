//! 模拟 Bot：用自己的令牌连总线，列出工具或调用一个工具。
//!   bot <令牌> list
//!   bot <令牌> call <工具> '<JSON 参数>'
//! 总线地址默认 http://127.0.0.1:8100/mcp，可用环境变量 MCP_URL 指向其他候选（如 Docker MCP Gateway）。

use rmcp::model::{
    CallToolRequestParams, ClientCapabilities, ClientConfig, Implementation, ProtocolVersion,
};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::{ClientLifecycleMode, ClientServiceExt};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [token, action, rest @ ..] = args.as_slice() else {
        anyhow::bail!("用法：bot <令牌> list | call <工具> '<JSON 参数>'");
    };
    let url = std::env::var("MCP_URL").unwrap_or_else(|_| "http://127.0.0.1:8100/mcp".into());
    let config = StreamableHttpClientTransportConfig::with_uri(url).auth_header(token.clone());
    let client = ClientConfig::new(ClientCapabilities::default(), Implementation::new("bot", "0"))
        .serve_with_lifecycle(
            StreamableHttpClientTransport::from_config(config),
            ClientLifecycleMode::Auto {
                preferred_versions: vec![ProtocolVersion::V_2026_07_28],
                legacy_version: Some(ProtocolVersion::V_2025_11_25),
            },
        )
        .await?;
    match (action.as_str(), rest) {
        ("list", _) => {
            let tools = client.list_tools(None).await?;
            let names: Vec<&str> = tools.tools.iter().map(|t| t.name.as_ref()).collect();
            println!("{}", serde_json::to_string(&names)?);
        }
        ("call", [tool, arguments]) => {
            let arguments: serde_json::Map<String, serde_json::Value> = serde_json::from_str(arguments)?;
            let params = CallToolRequestParams::new(tool.clone()).with_arguments(arguments);
            match client.call_tool(params).await {
                Ok(result) => println!("{}", serde_json::to_string(&result)?),
                Err(e) => println!("{{\"error\": {:?}}}", e.to_string()),
            }
        }
        _ => anyhow::bail!("未知操作 {action}"),
    }
    client.cancel().await?;
    Ok(())
}
