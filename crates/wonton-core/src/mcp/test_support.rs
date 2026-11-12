//! 总线测试的替身：一个说 MCP 的假插件（笔记），和记下确认卡片的假通道。只存在于测试里。

use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use rmcp::ServerHandler;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, ErrorData,
    ListToolsResult, PaginatedRequestParams, ReadResourceRequestParams, ReadResourceResponse,
    ReadResourceResult, ResourceContents, ServerCapabilities, ServerConfig, Tool, ToolAnnotations,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use serde_json::json;
use tokio::net::TcpListener;
use wonton_protocol::{ConfirmationCard, ConfirmationState};

use super::Notifier;

/// 假插件：`search`、`find` 标了只读，`delete`、`archive` 没标（会改数据），`purge` 标了会删东西
/// （`destructiveHint`）。`deleted` 记下删过的笔记。
#[derive(Clone)]
pub struct Notes {
    pub deleted: Arc<Mutex<Vec<String>>>,
    /// 为真时工具一直不回答。
    pub hang: Arc<std::sync::atomic::AtomicBool>,
    /// 调用方取消了几次还在跑的调用。
    pub cancelled: Arc<std::sync::atomic::AtomicUsize>,
    /// 工具 `find`、`archive` 返回的结果。
    pub found: Arc<Mutex<CallToolResult>>,
    /// 每次调用收到的 `_meta`（没有时是 `None`）。
    pub received_meta: Arc<Mutex<Vec<Option<rmcp::model::MetaObject>>>>,
}

impl Default for Notes {
    fn default() -> Self {
        Self {
            deleted: Arc::default(),
            hang: Arc::default(),
            cancelled: Arc::default(),
            found: Arc::new(Mutex::new(CallToolResult::success(Vec::new()))),
            received_meta: Arc::default(),
        }
    }
}

fn schema() -> Arc<rmcp::model::JsonObject> {
    let value = json!({"type": "object", "properties": {"name": {"type": "string"}}});
    Arc::new(value.as_object().cloned().unwrap_or_default())
}

impl ServerHandler for Notes {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
        )
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _ctx: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let search = Tool::new("search", "找笔记", schema())
            .with_annotations(ToolAnnotations::new().read_only(true))
            .with_meta(rmcp::model::MetaObject(
                json!({"ui": {"visibility": ["app"]}})
                    .as_object()
                    .cloned()
                    .unwrap_or_default(),
            ));
        let delete = Tool::new("delete", "删笔记", schema()).with_meta(rmcp::model::MetaObject(
            json!({"ui": {"visibility": ["app"]}})
                .as_object()
                .cloned()
                .unwrap_or_default(),
        ));
        let find = Tool::new("find", "找文件", schema())
            .with_annotations(ToolAnnotations::new().read_only(true));
        let archive = Tool::new("archive", "归档文件", schema());
        let boom = Tool::new("boom", "总是报错", schema())
            .with_annotations(ToolAnnotations::new().read_only(true));
        let purge = Tool::new("purge", "彻底删掉笔记", schema())
            .with_annotations(ToolAnnotations::new().read_only(false).destructive(true));
        Ok(ListToolsResult::with_all_items(vec![
            search, delete, find, archive, boom, purge,
        ]))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        if self.hang.load(std::sync::atomic::Ordering::SeqCst) {
            ctx.ct.cancelled().await;
            self.cancelled
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            return Err(ErrorData::internal_error("取消了", None));
        }
        // rmcp 把进站请求的 `_meta` 挪进 RequestContext（typed 字段只在出站时用）。
        self.received_meta
            .lock()
            .unwrap()
            .push(Some(ctx.meta.0.clone()));
        let name = request
            .arguments
            .as_ref()
            .and_then(|a| a.get("name"))
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_owned();
        let text = match request.name.as_ref() {
            "boom" => return Err(ErrorData::internal_error("炸了", None)),
            "find" | "archive" => return Ok(self.found.lock().unwrap().clone().into()),
            "search" => format!("找到 {name}.txt"),
            "delete" | "purge" => {
                self.deleted.lock().unwrap().push(name.clone());
                format!("删掉了 {name}.txt")
            }
            other => return Err(ErrorData::invalid_params(format!("没有 {other}"), None)),
        };
        Ok(CallToolResult::success(vec![ContentBlock::text(text)]).into())
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _ctx: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, ErrorData> {
        Ok(ReadResourceResult::new(vec![resource(&request.uri)?]).into())
    }
}

/// 假插件的资源：`notes://text` 是文字，`notes://blob` 是 base64 的 `abc`，`notes://huge` 超过 `resources/read` 的上限，其余
/// 读不了。
fn resource(uri: &str) -> Result<ResourceContents, ErrorData> {
    match uri {
        "notes://text" => Ok(ResourceContents::text("一条笔记", uri)),
        "ui://monitor" => Ok(ResourceContents::TextResourceContents {
            uri: uri.to_owned(),
            mime_type: Some("text/html;profile=mcp-app".to_owned()),
            text: "<p>ok</p>".to_owned(),
            meta: Some(rmcp::model::MetaObject(
                json!({"ui": {"csp": {"connectDomains": ["https://api.example.com"]}}})
                    .as_object()
                    .cloned()
                    .unwrap_or_default(),
            )),
        }),
        "notes://huge" => Ok(ResourceContents::text(
            "x".repeat(crate::files::MAX_RESOURCE_BYTES + 1),
            uri,
        )),
        "notes://blob" => Ok(ResourceContents::BlobResourceContents {
            uri: uri.to_owned(),
            mime_type: None,
            blob: "YWJj".to_owned(),
            meta: None,
        }),
        _ => Err(ErrorData::resource_not_found(format!("没有 {uri}"), None)),
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
    /// 每次更新用的标识。
    pub update_handles: Mutex<Vec<String>>,
    /// 下一次更新时卡片换了地方，给出这个新标识。
    pub moves_to: Mutex<Option<String>>,
    /// 为真时发卡片和更新卡片都失败，模拟 Matrix 不可用。
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
        handle: &'a str,
        card: &'a ConfirmationCard,
    ) -> BoxFuture<'a, Result<Option<String>, String>> {
        async move {
            if self.broken.load(std::sync::atomic::Ordering::SeqCst) {
                return Err("Matrix 连不上".to_owned());
            }
            self.updates.lock().unwrap().push(card.clone());
            self.update_handles.lock().unwrap().push(handle.to_owned());
            Ok(self.moves_to.lock().unwrap().take())
        }
        .boxed()
    }
}
