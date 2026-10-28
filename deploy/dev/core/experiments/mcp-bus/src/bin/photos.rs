//! 假的“相册插件” MCP 服务器（streamable HTTP）：一个普通工具、一个敏感工具。
//! 三个总线候选都连它，作为同一个上游。用法：photos [监听地址，默认 0.0.0.0:8101]

use rmcp::handler::server::wrapper::Parameters;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{schemars, tool, tool_router};

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct SearchRequest {
    /// 搜索内容，如“海边”
    query: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct DeleteRequest {
    /// 照片 ID
    id: String,
}

#[derive(Debug, Clone)]
struct Photos;

#[tool_router(server_handler)]
impl Photos {
    #[tool(description = "按内容搜索照片")]
    fn search_photos(&self, Parameters(SearchRequest { query }): Parameters<SearchRequest>) -> String {
        serde_json::json!([{"id": "p1", "caption": format!("{query}（示例）")}]).to_string()
    }

    #[tool(description = "删除一张照片")]
    fn delete_photo(&self, Parameters(DeleteRequest { id }): Parameters<DeleteRequest>) -> String {
        format!("已删除 {id}")
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let addr = std::env::args().nth(1).unwrap_or_else(|| "0.0.0.0:8101".into());
    // 实验环境里容器经 host.docker.internal 访问它，放开 Host 校验。
    let config = StreamableHttpServerConfig::default().disable_allowed_hosts();
    let service = StreamableHttpService::new(|| Ok(Photos), LocalSessionManager::default().into(), config);
    let router = axum::Router::new().nest_service("/mcp", service);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    eprintln!("photos MCP 服务器：http://{addr}/mcp");
    axum::serve(listener, router).await?;
    Ok(())
}
