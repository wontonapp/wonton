//! MCP 总线（ADR-0006、ADR-0016）：Agent 经 `mcp.wonton.internal` 连上来，看到核心自己的工具和所有在运行的
//! 插件声明的工具。调用方由网关按来源地址认出（[`Caller`]）；没标只读的工具先请主人确认（[`confirm`]）；每次
//! 调用写一行审计（[`audit`]）。

pub mod audit;
mod bounded_http;
mod cards;
mod confirm;
mod core_tools;
mod file_results;
mod file_tools;
mod naming;
mod notifier;
mod server;
pub mod upstream;

use std::future::Future;
use std::sync::{Arc, Mutex, OnceLock};

use rmcp::model::{
    CallToolRequestParams, CallToolResult, ContentBlock, JsonObject, MetaObject, Tool,
};
use rmcp::service::{Peer, RoleServer};
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use serde_json::Value;
use wonton_protocol::{ConfirmationState, FileFailure};

use crate::config::PLUGIN_MCP_HOST;
use crate::files::{BotRef, Failure, Files};
use crate::plugin::{McpServer, PluginId, PluginManager};
use audit::{Audit, AuditedFile, Decision, Record};
use cards::Draft;
pub use cards::content as card_content;
pub use confirm::{AnswerError, Confirmations, Notifier};
pub use notifier::NoMatrix;
pub use server::Caller;
use upstream::{CallError, UncertainKind, Upstreams};

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
    upstreams: Arc<Upstreams>,
    audit: Arc<Audit>,
    /// 文件服务。它读插件的资源要用这里的 [`Upstreams`]，所以总线先建好，文件服务后接上（[`Bus::attach_files`]）。
    files: OnceLock<Arc<Files>>,
    /// 旧协议（2026-07-28 之前）的会话：工具列表变了要逐个通知。
    peers: Mutex<Vec<Peer<RoleServer>>>,
}

/// 一次执行的结果：返回给 Agent 的内容，和卡片的最终状态。
struct Outcome {
    result: CallToolResult,
    state: ConfirmationState,
    /// 这次调用登记的文件，写进审计。
    files: Vec<AuditedFile>,
}

impl Outcome {
    fn ok(text: String, message: String) -> Self {
        Self {
            result: CallToolResult::success(vec![ContentBlock::text(text)]),
            state: ConfirmationState::Succeeded { message },
            files: Vec::new(),
        }
    }

    fn failed(message: String) -> Self {
        Self {
            result: error(&message),
            state: ConfirmationState::Failed { message },
            files: Vec::new(),
        }
    }
}

/// 转给插件。结果里的文件引用换成编号（[`file_results`]）。
async fn forward(
    bus: &Arc<Bus>,
    caller: &Caller,
    server: &McpServer,
    tool: &str,
    arguments: JsonObject,
    cancelled: impl Future<Output = ()> + Send,
) -> Outcome {
    let called = bus
        .upstreams
        .call(server, tool, Some(arguments), cancelled)
        .await;
    match called {
        Ok(result) => {
            let state = if result.is_error == Some(true) {
                ConfirmationState::Failed {
                    message: "工具报告执行失败".to_owned(),
                }
            } else {
                ConfirmationState::Succeeded {
                    message: "已执行".to_owned(),
                }
            };
            let (result, files) = file_results::rewrite(bus, caller, &server.id, result).await;
            Outcome {
                result,
                state,
                files,
            }
        }
        Err(CallError::Uncertain { kind, message }) => Outcome {
            result: uncertain(&message, kind),
            state: ConfirmationState::Uncertain { message },
            files: Vec::new(),
        },
        Err(e) => Outcome::failed(e.to_string()),
    }
}

/// 工具结果里给 Agent 看的类别，结果不确定时才有：和工具自己报的错分开，Agent 不该直接重试。
const FAILURE_META: &str = "app.wonton/failure";

fn uncertain(message: &str, kind: UncertainKind) -> CallToolResult {
    let mut result = error(message);
    let meta = [(FAILURE_META.to_owned(), Value::from(kind.as_str()))];
    result.meta = Some(MetaObject(meta.into_iter().collect()));
    result
}

/// 工具执行失败：Agent 能读到原因（MCP 的 `isError`）。
fn error(message: &str) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(message.to_owned())])
}

impl Bus {
    pub fn new(
        plugins: Arc<PluginManager>,
        confirmations: Arc<Confirmations>,
        audit: Arc<Audit>,
    ) -> Arc<Self> {
        Self::with_upstreams(plugins, confirmations, audit, Arc::new(Upstreams::new()))
    }

    fn with_upstreams(
        plugins: Arc<PluginManager>,
        confirmations: Arc<Confirmations>,
        audit: Arc<Audit>,
        upstreams: Arc<Upstreams>,
    ) -> Arc<Self> {
        let bus = Arc::new(Self {
            plugins,
            confirmations,
            upstreams,
            audit,
            files: OnceLock::new(),
            peers: Mutex::new(Vec::new()),
        });
        bus.watch_plugins();
        bus
    }

    /// 接上文件服务。没接上时工具结果里的文件登记不了，Bot 的文件工具报 `service_unavailable`。
    pub fn attach_files(&self, files: Arc<Files>) {
        if self.files.set(files).is_err() {
            tracing::error!("文件服务接了两次，用先接上的");
        }
    }

    fn files(&self) -> Result<&Arc<Files>, Failure> {
        self.files
            .get()
            .ok_or_else(|| Failure::new(FileFailure::ServiceUnavailable, "核心没有启用文件服务"))
    }

    /// 文件访问范围里的这个调用方：有 Bot 账号的插件是它的 Matrix 用户名，和 `grants` 查到的一样；没有的（比如不进聊天的 Agent）是插件
    /// ID，它有点，用户名里没有，不会重名。
    fn bot_name(&self, caller: &Caller) -> Result<String, Failure> {
        let name = self
            .plugins
            .bot_name(&caller.id)
            .map_err(|e| Failure::new(FileFailure::Internal, e.to_string()))?;
        Ok(name.unwrap_or_else(|| caller.id.to_string()))
    }

    fn bot_ref(&self, caller: &Caller, plugin: &PluginId) -> Result<BotRef, Failure> {
        Ok(BotRef {
            name: self.bot_name(caller)?,
            plugin: plugin.clone(),
        })
    }

    /// 插件的 MCP 客户端：文件服务读资源也用它，一个插件一条连接。
    pub fn upstreams(&self) -> Arc<Upstreams> {
        self.upstreams.clone()
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
                bus.upstreams.retain(&bus.plugins.mcp_servers());
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

    /// 调用方能看到的工具：核心自己的，加上所有在运行的插件的。并发地问，一个插件卡住挡不住别的（每个插件有期限，见
    /// [`Upstreams::tools`]）。列不出工具的插件记一条警告后跳过，它的工具这时本来也调不通。
    async fn tools(&self, caller: &Caller) -> Vec<Tool> {
        let servers = self.plugins.mcp_servers();
        let listed = servers.iter().map(|server| self.upstreams.tools(server));
        let listed = futures_util::future::join_all(listed).await;
        let mut tools = core_tools::definitions();
        for (server, list) in servers.iter().zip(listed) {
            match list {
                Ok(list) => tools.extend(list.into_iter().map(|mut tool| {
                    tool.name = naming::join(&server.id.slug(), &tool.name).into();
                    tool
                })),
                Err(e) => tracing::warn!(caller = %caller.id, error = %e, "插件的工具暂时不可用"),
            }
        }
        tools
    }

    /// `cancelled` 完成时调用方已经不要结果了（取消了，或者断开了）。
    async fn call(
        self: &Arc<Self>,
        caller: &Caller,
        request: CallToolRequestParams,
        cancelled: impl Future<Output = ()> + Send,
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
        self.call_plugin(caller, &server, &name, tool, arguments, cancelled)
            .await
    }

    async fn call_plugin(
        self: &Arc<Self>,
        caller: &Caller,
        server: &McpServer,
        name: &str,
        tool: &str,
        arguments: JsonObject,
        cancelled: impl Future<Output = ()> + Send,
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
        if read_only {
            let outcome = forward(self, caller, server, tool, arguments, cancelled).await;
            self.record(caller, name, &args, Decision::Allowed, &outcome);
            return outcome.result;
        }
        let plugin = cards::text(&server.title);
        let draft = cards::tool_call(&caller.name, &plugin, tool, &args);
        // 不传 `cancelled`：确认过的调用不跟着 Agent 走，它断开了照样执行，结果在卡片上（ADR-0016）。
        let (agent, server, tool) = (caller.clone(), server.clone(), tool.to_owned());
        let run = move |bus: Arc<Bus>| async move {
            forward(
                &bus,
                &agent,
                &server,
                &tool,
                arguments,
                std::future::pending(),
            )
            .await
        };
        self.confirmed(caller, name, args, draft, run).await
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
                        files: &[],
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
            files: &[],
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
        let files = &outcome.files;
        let outcome = match &outcome.state {
            ConfirmationState::Failed { message } | ConfirmationState::Uncertain { message } => {
                message.as_str()
            }
            _ => "ok",
        };
        self.audit.record(&Record {
            caller: caller.id.as_str(),
            tool: name,
            arguments: args,
            decision,
            outcome: Some(outcome),
            files,
        });
    }
}

#[cfg(test)]
mod file_tests;
#[cfg(test)]
pub(crate) mod test_support;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod upstream_tests;
