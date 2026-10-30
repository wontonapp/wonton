//! 总线测试的替身：一个说 MCP 的假插件（笔记），和记下确认卡片的假通道。只存在于测试里。

use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use rmcp::ServerHandler;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, ErrorData,
    ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool,
    ToolAnnotations,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use serde_json::json;
use tokio::net::TcpListener;
use wonton_protocol::{ConfirmationCard, ConfirmationState};

use super::Notifier;

/// 假插件：`search` 标了只读，`delete` 没标（会改数据）。`deleted` 记下删过的笔记。
#[derive(Clone, Default)]
pub struct Notes {
    pub deleted: Arc<Mutex<Vec<String>>>,
}

fn schema() -> Arc<rmcp::model::JsonObject> {
    let value = json!({"type": "object", "properties": {"name": {"type": "string"}}});
    Arc::new(value.as_object().cloned().unwrap_or_default())
}

impl ServerHandler for Notes {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _ctx: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let search = Tool::new("search", "找笔记", schema())
            .with_annotations(ToolAnnotations::new().read_only(true));
        let delete = Tool::new("delete", "删笔记", schema());
        Ok(ListToolsResult::with_all_items(vec![search, delete]))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let name = request
            .arguments
            .as_ref()
            .and_then(|a| a.get("name"))
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_owned();
        let text = match request.name.as_ref() {
            "search" => format!("找到 {name}.txt"),
            "delete" => {
                self.deleted.lock().unwrap().push(name.clone());
                format!("删掉了 {name}.txt")
            }
            other => return Err(ErrorData::invalid_params(format!("没有 {other}"), None)),
        };
        Ok(CallToolResult::success(vec![ContentBlock::text(text)]).into())
    }
}

/// 在本机起假插件的 MCP 服务，返回它的地址（`http://127.0.0.1:端口/mcp`）。
pub async fn serve_notes(notes: Notes) -> String {
    let config = StreamableHttpServerConfig::default().disable_allowed_hosts();
    let service = StreamableHttpService::new(
        move || Ok(notes.clone()),
        LocalSessionManager::default().into(),
        config,
    );
    let router = axum::Router::new().nest_service("/mcp", service);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    format!("http://{addr}/mcp")
}

/// 假的卡片通道：记下发出的卡片和每次更新。
#[derive(Default)]
pub struct Cards {
    pub posted: Mutex<Vec<ConfirmationCard>>,
    pub updates: Mutex<Vec<ConfirmationCard>>,
    /// 为真时发卡片失败，模拟 Matrix 不可用。
    pub broken: std::sync::atomic::AtomicBool,
}

impl Cards {
    /// 等第 `n` 张卡片（从 1 数）发出来。
    pub async fn wait_for(&self, n: usize) -> ConfirmationCard {
        for _ in 0..500 {
            if let Some(card) = self.posted.lock().unwrap().get(n - 1) {
                return card.clone();
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("等了 5 秒还没有第 {n} 张卡片");
    }

    /// 卡片 `id` 经历过的状态，按更新的顺序。
    pub fn states(&self, id: &str) -> Vec<ConfirmationState> {
        let updates = self.updates.lock().unwrap();
        updates
            .iter()
            .filter(|card| card.id == id)
            .map(|card| card.state.clone())
            .collect()
    }
}

impl Notifier for Cards {
    fn post<'a>(&'a self, card: &'a ConfirmationCard) -> BoxFuture<'a, Result<String, String>> {
        async move {
            if self.broken.load(std::sync::atomic::Ordering::SeqCst) {
                return Err("Matrix 连不上".to_owned());
            }
            let mut posted = self.posted.lock().unwrap();
            posted.push(card.clone());
            Ok(format!("$card{}", posted.len()))
        }
        .boxed()
    }

    fn update<'a>(
        &'a self,
        _handle: &'a str,
        card: &'a ConfirmationCard,
    ) -> BoxFuture<'a, Result<(), String>> {
        async move {
            self.updates.lock().unwrap().push(card.clone());
            Ok(())
        }
        .boxed()
    }
}
