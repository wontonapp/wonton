//! MCP 总线（ADR-0006、ADR-0016）：Agent 经 `mcp.wonton.internal` 连上来，看到核心自己的工具和所有在运行的
//! 插件声明的工具。调用方由网关按来源地址认出（[`Caller`]）；没标只读的工具先请主人确认（[`confirm`]）；每次
//! 调用写一行审计（[`audit`]）。

mod audit;
mod cards;
mod confirm;
mod core_tools;
mod naming;
mod notifier;
mod server;
mod upstream;

use std::future::Future;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use rmcp::model::{CallToolRequestParams, CallToolResult, ContentBlock, JsonObject, Tool};
use rmcp::service::{Peer, RoleServer};
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use serde_json::Value;
use wonton_protocol::ConfirmationState;

use crate::config::PLUGIN_MCP_HOST;
use crate::plugin::{McpServer, PluginManager};
use audit::{Audit, Decision, Record};
use cards::Draft;
pub use cards::content as card_content;
pub use confirm::{AnswerError, Confirmations, Notifier};
pub use notifier::NoMatrix;
pub use server::Caller;
use upstream::Upstreams;

/// 挂到网关上的 Streamable HTTP 服务。
pub type McpService = StreamableHttpService<server::Handler, LocalSessionManager>;

/// 网关用的总线入口：给 Agent 的 MCP 服务，和收主人回答的确认表。
#[derive(Clone)]
pub struct Endpoint {
    pub bus: Arc<Bus>,
    pub service: McpService,
}

pub struct Bus {
    plugins: Arc<PluginManager>,
    confirmations: Arc<Confirmations>,
    upstreams: Upstreams,
    audit: Audit,
    /// 旧协议（2026-07-28 之前）的会话：工具列表变了要逐个通知。
    peers: Mutex<Vec<Peer<RoleServer>>>,
}

/// 一次执行的结果：返回给 Agent 的内容，和卡片的最终状态。
struct Outcome {
    result: CallToolResult,
    state: ConfirmationState,
}

impl Outcome {
    fn ok(text: String, message: String) -> Self {
        Self {
            result: CallToolResult::success(vec![ContentBlock::text(text)]),
            state: ConfirmationState::Succeeded { message },
        }
    }

    fn failed(message: String) -> Self {
        Self {
            result: error(&message),
            state: ConfirmationState::Failed { message },
        }
    }
}

/// 工具执行失败：Agent 能读到原因（MCP 的 `isError`）。
fn error(message: &str) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(message.to_owned())])
}

impl Bus {
    pub fn new(
        plugins: Arc<PluginManager>,
        confirmations: Arc<Confirmations>,
        audit_path: PathBuf,
    ) -> Arc<Self> {
        let bus = Arc::new(Self {
            plugins,
            confirmations,
            upstreams: Upstreams::new(),
            audit: Audit::new(audit_path),
            peers: Mutex::new(Vec::new()),
        });
        bus.watch_plugins();
        bus
    }

    pub fn confirmations(&self) -> &Confirmations {
        &self.confirmations
    }

    pub fn endpoint(self: &Arc<Self>) -> Endpoint {
        Endpoint {
            bus: self.clone(),
            service: self.service(),
        }
    }

    fn service(self: &Arc<Self>) -> McpService {
        let bus = self.clone();
        // Host 已经由网关校验过；Agent 不是浏览器，带 Origin 的请求一律拒绝。
        let config = StreamableHttpServerConfig::default()
            .with_allowed_hosts([PLUGIN_MCP_HOST])
            .enforce_origin_validation();
        StreamableHttpService::new(
            move || Ok(server::Handler::new(bus.clone())),
            LocalSessionManager::default().into(),
            config,
        )
    }

    /// 插件表变了：丢掉不再运行的插件的客户端，告诉旧协议的会话工具列表变了（新协议的经 `subscriptions/listen`，
    /// 见 [`server::Handler`]）。
    fn watch_plugins(self: &Arc<Self>) {
        let bus = Arc::downgrade(self);
        let mut changes = self.plugins.changes();
        tokio::spawn(async move {
            while changes.changed().await.is_ok() {
                let Some(bus) = bus.upgrade() else { break };
                bus.upstreams.retain(&bus.plugins.mcp_servers()).await;
                let peers: Vec<_> = bus.peers.lock().expect("会话表锁中毒").clone();
                let mut alive = Vec::new();
                for peer in peers {
                    if peer.notify_tool_list_changed().await.is_ok() {
                        alive.push(peer);
                    }
                }
                *bus.peers.lock().expect("会话表锁中毒") = alive;
            }
        });
    }

    fn track(&self, peer: Peer<RoleServer>) {
        self.peers.lock().expect("会话表锁中毒").push(peer);
    }

    /// 调用方能看到的工具：核心自己的，加上所有在运行的插件的。列不出工具的插件记一条警告后跳过，它的工具
    /// 这时本来也调不通。
    async fn tools(&self, caller: &Caller) -> Vec<Tool> {
        let mut tools = core_tools::definitions();
        for server in self.plugins.mcp_servers() {
            match self.upstreams.tools(&server).await {
                Ok(list) => tools.extend(list.into_iter().map(|mut tool| {
                    tool.name = naming::join(&server.id.slug(), &tool.name).into();
                    tool
                })),
                Err(e) => tracing::warn!(caller = %caller.id, error = %e, "插件的工具暂时不可用"),
            }
        }
        tools
    }

    async fn call(
        self: &Arc<Self>,
        caller: &Caller,
        request: CallToolRequestParams,
    ) -> CallToolResult {
        let name = request.name.to_string();
        let arguments = request.arguments.unwrap_or_default();
        let args = Value::Object(arguments.clone());
        let Some((owner, tool)) = naming::split(&name) else {
            return self.deny(caller, &name, &args, &format!("没有工具 {name}"));
        };
        if owner == naming::CORE {
            return core_tools::call(self, caller, tool, arguments).await;
        }
        let Some(server) = self
            .plugins
            .mcp_servers()
            .into_iter()
            .find(|s| s.id.slug() == owner)
        else {
            return self.deny(
                caller,
                &name,
                &args,
                &format!("没有在运行的插件提供 {name}"),
            );
        };
        self.call_plugin(caller, &server, &name, tool, arguments)
            .await
    }

    async fn call_plugin(
        self: &Arc<Self>,
        caller: &Caller,
        server: &McpServer,
        name: &str,
        tool: &str,
        arguments: JsonObject,
    ) -> CallToolResult {
        let args = Value::Object(arguments.clone());
        let definition = match self.upstreams.tools(server).await {
            Ok(tools) => tools.into_iter().find(|t| t.name == tool),
            Err(e) => return self.deny(caller, name, &args, &e),
        };
        let Some(definition) = definition else {
            return self.deny(
                caller,
                name,
                &args,
                &format!("插件 {} 没有工具 {tool}", server.id),
            );
        };
        let read_only = definition
            .annotations
            .as_ref()
            .and_then(|a| a.read_only_hint)
            == Some(true);
        let forward = {
            let server = server.clone();
            let tool = tool.to_owned();
            move |bus: Arc<Bus>| async move {
                match bus.upstreams.call(&server, &tool, Some(arguments)).await {
                    Ok(result) if result.is_error == Some(true) => Outcome {
                        result,
                        state: ConfirmationState::Failed {
                            message: "工具报告执行失败".to_owned(),
                        },
                    },
                    Ok(result) => Outcome {
                        result,
                        state: ConfirmationState::Succeeded {
                            message: "已执行".to_owned(),
                        },
                    },
                    Err(e) => Outcome::failed(e),
                }
            }
        };
        if read_only {
            let outcome = forward(self.clone()).await;
            self.record(caller, name, &args, Decision::Allowed, &outcome);
            return outcome.result;
        }
        let plugin = cards::text(&server.title);
        let draft = cards::tool_call(&caller.name, &plugin, tool, &args);
        self.confirmed(caller, name, args, draft, forward).await
    }

    /// 请主人确认，确认了再执行 `run`。整个过程在后台任务里跑：Agent 等不及断开了，确认照样有效到过期，主人确认后
    /// 照样执行，结果在卡片上（ADR-0016）。
    async fn confirmed<F, Fut>(
        self: &Arc<Self>,
        caller: &Caller,
        name: &str,
        args: Value,
        draft: Draft,
        run: F,
    ) -> CallToolResult
    where
        F: FnOnce(Arc<Bus>) -> Fut + Send + 'static,
        Fut: Future<Output = Outcome> + Send,
    {
        let bus = self.clone();
        let caller = caller.clone();
        let name = name.to_owned();
        let task = tokio::spawn(async move {
            let decision = bus.confirmations.ask(&caller.id, draft).await;
            match decision {
                Ok(ticket) => {
                    let outcome = run(bus.clone()).await;
                    bus.record(&caller, &name, &args, Decision::Approved, &outcome);
                    bus.confirmations.finish(ticket, outcome.state).await;
                    outcome.result
                }
                Err(refusal) => {
                    let decision = match refusal {
                        confirm::Refusal::Rejected => Decision::Rejected,
                        confirm::Refusal::Expired => Decision::Expired,
                        _ => Decision::Unconfirmed,
                    };
                    let message = refusal.message();
                    bus.audit.record(&Record {
                        caller: caller.id.as_str(),
                        tool: &name,
                        arguments: &args,
                        decision,
                        outcome: Some(&message),
                    });
                    error(&message)
                }
            }
        });
        task.await
            .unwrap_or_else(|e| error(&format!("确认流程意外中止：{e}")))
    }

    fn deny(&self, caller: &Caller, name: &str, args: &Value, message: &str) -> CallToolResult {
        self.audit.record(&Record {
            caller: caller.id.as_str(),
            tool: name,
            arguments: args,
            decision: Decision::Denied,
            outcome: Some(message),
        });
        error(message)
    }

    fn record(
        &self,
        caller: &Caller,
        name: &str,
        args: &Value,
        decision: Decision,
        outcome: &Outcome,
    ) {
        let outcome = match &outcome.state {
            ConfirmationState::Failed { message } => message.as_str(),
            _ => "ok",
        };
        self.audit.record(&Record {
            caller: caller.id.as_str(),
            tool: name,
            arguments: args,
            decision,
            outcome: Some(outcome),
        });
    }
}

#[cfg(test)]
pub(crate) mod test_support;
#[cfg(test)]
mod tests;
