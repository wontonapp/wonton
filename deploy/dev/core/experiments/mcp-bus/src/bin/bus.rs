//! 最小 MCP 总线（实验）：Bot 用自己的令牌连进来，总线把插件的工具汇总成一个 MCP 服务器，
//! 按 Bot 白名单过滤 `tools/list`、拦截 `tools/call`；敏感工具先挂起，等主人确认
//! （这里用 `POST /approvals/{id}?approve=true` 代替聊天里的确认卡片）；每次调用写一行审计日志。
//!
//! 用法：bus [上游 MCP 地址，默认 http://127.0.0.1:8101/mcp]，监听 127.0.0.1:8100。

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::extract::{Path, Query, State};
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ClientCapabilities, ClientConfig,
    ContentBlock, ErrorData, Implementation, ListToolsResult, PaginatedRequestParams,
    ProtocolVersion, ServerCapabilities, ServerConfig,
};
use rmcp::service::{Peer, RequestContext, RoleClient, RoleServer};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{ClientLifecycleMode, ClientServiceExt, ServerHandler};
use serde_json::json;
use tokio::io::AsyncWriteExt;
use tokio::sync::{Mutex, oneshot};

/// 插件工具在总线上的名字：`<插件>__<工具>`，避免不同插件的同名工具冲突。
const PREFIX: &str = "photos__";
/// 改变状态的工具：不直接执行，先等主人确认。
const SENSITIVE: &[&str] = &["photos__delete_photo"];

struct Bot {
    name: &'static str,
    allow: &'static [&'static str],
}

/// 每个 Bot 一个令牌、一份工具白名单。产品里由核心在创建 Bot 时签发。
fn bot_for(token: &str) -> Option<Bot> {
    match token {
        "bot-owner-assistant" => Some(Bot {
            name: "主人助手",
            allow: &["photos__search_photos", "photos__delete_photo"],
        }),
        "bot-family-guest" => Some(Bot {
            name: "访客 Bot",
            allow: &["photos__search_photos"],
        }),
        _ => None,
    }
}

#[derive(Clone)]
struct Bus {
    upstream: Peer<RoleClient>,
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<bool>>>>,
    audit: Arc<Mutex<tokio::fs::File>>,
}

impl Bus {
    fn bot(&self, ctx: &RequestContext<RoleServer>) -> Result<Bot, ErrorData> {
        let token = ctx
            .extensions
            .get::<http::request::Parts>()
            .and_then(|parts| parts.headers.get(http::header::AUTHORIZATION))
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .unwrap_or_default();
        bot_for(token).ok_or_else(|| ErrorData::invalid_request("未知的 Bot 令牌", None))
    }

    async fn audit(&self, bot: &str, tool: &str, decision: &str, detail: serde_json::Value) {
        let ts = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
        let line = json!({"ts": ts, "bot": bot, "tool": tool, "decision": decision, "detail": detail});
        let mut file = self.audit.lock().await;
        let _ = file.write_all(format!("{line}\n").as_bytes()).await;
        let _ = file.flush().await;
        eprintln!("审计：{line}");
    }

    /// 挂起敏感调用，直到主人确认或超时（产品里是聊天中的确认卡片）。
    async fn confirm(&self, bot: &Bot, tool: &str) -> bool {
        let id = format!("{:x}", SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos());
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id.clone(), tx);
        self.audit(bot.name, tool, "awaiting_confirmation", json!({"approval": id})).await;
        eprintln!("等待主人确认：curl -X POST 'http://127.0.0.1:8100/approvals/{id}?approve=true'");
        let approved = matches!(tokio::time::timeout(Duration::from_secs(120), rx).await, Ok(Ok(true)));
        self.pending.lock().await.remove(&id);
        approved
    }
}

impl ServerHandler for Bus {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("wonton-mcp-bus-spike", "0.0.0"))
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let bot = self.bot(&ctx)?;
        let upstream = self
            .upstream
            .list_tools(None)
            .await
            .map_err(|e| ErrorData::internal_error(format!("插件不可达：{e}"), None))?;
        let tools = upstream
            .tools
            .into_iter()
            .map(|mut tool| {
                tool.name = format!("{PREFIX}{}", tool.name).into();
                tool
            })
            .filter(|tool| bot.allow.contains(&tool.name.as_ref()))
            .collect();
        Ok(ListToolsResult::with_all_items(tools))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let bot = self.bot(&ctx)?;
        let tool = request.name.to_string();
        let args = json!(request.arguments);
        if !bot.allow.contains(&tool.as_str()) {
            self.audit(bot.name, &tool, "denied", args).await;
            return Err(ErrorData::invalid_request(format!("{} 无权调用 {tool}", bot.name), None));
        }
        if SENSITIVE.contains(&tool.as_str()) && !self.confirm(&bot, &tool).await {
            self.audit(bot.name, &tool, "rejected_by_owner", args).await;
            return Ok(CallToolResult::error(vec![ContentBlock::text("主人没有确认这次操作")]).into());
        }
        let mut forwarded = request.clone();
        forwarded.name = tool.trim_start_matches(PREFIX).to_owned().into();
        let result = self
            .upstream
            .call_tool(forwarded)
            .await
            .map_err(|e| ErrorData::internal_error(format!("插件调用失败：{e}"), None))?;
        let decision = if result.is_error == Some(true) { "failed" } else { "ok" };
        self.audit(bot.name, &tool, decision, args).await;
        Ok(result.into())
    }
}

async fn approve(
    State(pending): State<Arc<Mutex<HashMap<String, oneshot::Sender<bool>>>>>,
    Path(id): Path<String>,
    Query(query): Query<HashMap<String, String>>,
) -> &'static str {
    let approve = query.get("approve").is_some_and(|v| v == "true");
    let sender = pending.lock().await.remove(&id);
    match sender.map(|tx| tx.send(approve)) {
        Some(Ok(())) => "已提交\n",
        _ => "没有这个待确认请求\n",
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let upstream_url = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "http://127.0.0.1:8101/mcp".into());
    let client = ClientConfig::new(
        ClientCapabilities::default(),
        Implementation::new("wonton-mcp-bus-spike", "0.0.0"),
    )
    .serve_with_lifecycle(
        StreamableHttpClientTransport::from_uri(upstream_url.as_str()),
        ClientLifecycleMode::Auto {
            preferred_versions: vec![ProtocolVersion::V_2026_07_28],
            legacy_version: Some(ProtocolVersion::V_2025_11_25),
        },
    )
    .await?;
    let bus = Bus {
        upstream: client.peer().clone(),
        pending: Arc::default(),
        audit: Arc::new(Mutex::new(
            tokio::fs::OpenOptions::new().create(true).append(true).open("audit.jsonl").await?,
        )),
    };
    let pending = bus.pending.clone();
    let service = StreamableHttpService::new(
        move || Ok(bus.clone()),
        LocalSessionManager::default().into(),
        StreamableHttpServerConfig::default(),
    );
    let router = axum::Router::new()
        .route("/approvals/{id}", axum::routing::post(approve))
        .with_state(pending)
        .nest_service("/mcp", service);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:8100").await?;
    eprintln!("MCP 总线：http://127.0.0.1:8100/mcp（上游 {upstream_url}）");
    axum::serve(listener, router).await?;
    drop(client);
    Ok(())
}
