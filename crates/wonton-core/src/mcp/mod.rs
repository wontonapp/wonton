//! MCP 总线（ADR-0006、ADR-0016）：Agent 经 `mcp.wonton.internal` 连上来，看到核心自己的工具和所有在运行的
//! 插件声明的工具。调用方由网关按来源地址认出（[`Caller`]）；没标只读、主人也没放宽（ADR-0024）的工具先请主人确认
//! （[`confirm`]）；每次调用写一行审计（[`audit`]）。

mod agent_cards;
mod agent_install;
mod agent_permission;
mod apps;
mod audience;
pub mod audit;
mod authoring_tools;
mod bounded_http;
mod cards;
mod confirm;
mod confirmed;
mod contacts_gate;
mod core_tools;
mod dependencies;
mod file_results;
mod file_tools;
mod naming;
mod notifier;
mod panel_tools;
mod people_tools;
mod profile_tools;
mod risky;
mod save_tools;
mod server;
mod takeover;
mod tool_policy;
pub mod upstream;
pub(crate) mod windows;

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use rmcp::model::{CallToolRequestParams, CallToolResult, JsonObject, Tool};
use rmcp::service::{Peer, RoleServer};
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use serde_json::Value;
use wonton_protocol::{ConfirmationState, FileFailure, SecretValues};

use crate::bot_activity::Guard;
use crate::config::PLUGIN_MCP_HOST;
use crate::files::{BotRef, Failure, Files};
use crate::help::Help;
use crate::plugin::languages::Languages;
use crate::plugin::tool_policy::Clearance;
use crate::plugin::{McpServer, PluginId, PluginManager};
pub use agent_permission::AgentPermissionError;
use audience::Subject;
use audit::{Audit, Basis, Decision, Grounds, Record};
pub use cards::Draft;
pub use cards::text as card_text;
pub use confirm::{AnswerError, Audience, Confirmations, Notifier, Refusal};
pub use notifier::DevicePush;
pub use server::Caller;
use takeover::Takeovers;
pub use tool_policy::PolicyChange;
use upstream::Upstreams;

pub(crate) mod result;
pub(crate) use result::{Outcome, ToolFailure, error, forward, with_confirmation, with_failure};

/// 卡片过期之后，会话还要留多久：确认了才开始执行，执行的时间也算在这里。
const SESSION_IDLE_AFTER_CARD: Duration = Duration::from_secs(3600);

/// 接手申请的冷却（#817）：主人回绝或没理之后，同一个 Bot 对同一个插件这段时间内不再发接手卡片。
const ADOPT_COOLDOWN: Duration = Duration::from_secs(600);

/// 挂到网关上的 Streamable HTTP 服务。
pub type McpService = StreamableHttpService<server::Handler, LocalSessionManager>;

/// 网关用的总线入口：给 Agent 的 MCP 服务，和收主人回答的确认表。
#[derive(Clone)]
pub struct Endpoint {
    pub bus: Arc<Bus>,
    pub service: McpService,
}

/// `wonton__user_locale` 认人用的本服务器身份资料：Matrix 服务器名和家人记录。
/// Bot 钉页的卡片需要的 Matrix 入口。
pub struct Cards {
    matrix: Option<Arc<crate::matrix::Matrix>>,
    owner: String,
}

pub struct People {
    server_name: String,
    members: Arc<crate::members::Members>,
}

pub struct Bus {
    plugins: Arc<PluginManager>,
    confirmations: Arc<Confirmations>,
    upstreams: Arc<Upstreams>,
    audit: Arc<Audit>,
    takeovers: Takeovers,
    /// 接手申请的冷却（#817）：主人回绝或没理之后，同一个 Bot 对同一个插件在这段时间内不再发接手卡片。
    adopt_refusals: Mutex<HashMap<(PluginId, PluginId), Instant>>,
    /// 文件服务。它读插件的资源要用这里的 [`Upstreams`]，所以总线先建好，文件服务后接上（[`Bus::attach_files`]）。
    files: OnceLock<Arc<Files>>,
    /// 帮助文档。它要升级服务给的版本号，所以和文件服务一样后接上（[`Bus::attach_help`]）。
    help: OnceLock<Arc<Help>>,
    /// 家人记录和本服务器的 Matrix 服务器名：`wonton__user_locale` 认人用（[`Bus::attach_people`]）。
    people: OnceLock<People>,
    /// Matrix 和主人：Bot 钉进主面板的同意卡片要发进 Bot 的房间里（[`Bus::attach_matrix`]）。
    matrix: OnceLock<Cards>,
    /// Bot 的联系人：有远端联系人的 Bot 只聊天（[`Bus::attach_contacts`]，ADR-0050）。
    contacts: OnceLock<Arc<crate::contacts::Contacts>>,
    /// Bot 钉进主面板的同意卡片的等待：每个（插件）最多一张。
    pin_cards: Mutex<HashSet<PluginId>>,
    /// 旧协议（2026-07-28 之前）的会话：工具列表变了要逐个通知。
    peers: Mutex<Vec<Peer<RoleServer>>>,
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
        let recover = confirmations.has_leftover_windows();
        confirmations.attach_activity(plugins.activity.clone());
        let bus = Arc::new(Self {
            plugins,
            confirmations,
            upstreams,
            audit,
            takeovers: Takeovers::new(),
            adopt_refusals: Mutex::new(HashMap::new()),
            files: OnceLock::new(),
            help: OnceLock::new(),
            people: OnceLock::new(),
            matrix: OnceLock::new(),
            contacts: OnceLock::new(),
            pin_cards: Mutex::new(HashSet::new()),
            peers: Mutex::new(Vec::new()),
        });
        bus.watch_plugins();
        if recover {
            bus.recover_windows();
        }
        bus
    }

    /// 接上文件服务。没接上时工具结果里的文件登记不了，Bot 的文件工具报 `service_unavailable`。
    pub fn attach_files(&self, files: Arc<Files>) {
        if self.files.set(files).is_err() {
            tracing::error!("文件服务接了两次，用先接上的");
        }
    }

    /// 接上帮助文档。没接上时 `wonton__help` 如实报错。
    pub fn attach_help(&self, help: Arc<Help>) {
        if self.help.set(help).is_err() {
            tracing::error!("帮助文档接了两次，用先接上的");
        }
    }

    /// 接上家人记录和本服务器的 Matrix 服务器名（没接 Matrix 的核心是基础域名）。没接上时
    /// `wonton__user_locale` 如实报错。
    pub fn attach_people(&self, server_name: String, members: Arc<crate::members::Members>) {
        if self
            .people
            .set(People {
                server_name,
                members,
            })
            .is_err()
        {
            tracing::error!("家人记录接了两次，用先接上的");
        }
    }

    /// 接上 Matrix 和主人：Bot 往主面板钉页的同意卡片发在 Bot 自己的房间里。
    pub fn attach_matrix(&self, matrix: Option<Arc<crate::matrix::Matrix>>, owner: String) {
        let _ = self.matrix.set(Cards { matrix, owner });
    }

    fn cards(&self) -> Result<&Cards, String> {
        self.matrix
            .get()
            .ok_or_else(|| "service_unavailable: the core has no Matrix connection".to_owned())
    }

    fn people(&self) -> Result<&People, String> {
        self.people
            .get()
            .ok_or_else(|| "service_unavailable: the core has no family records".to_owned())
    }

    fn files(&self) -> Result<&Arc<Files>, Failure> {
        self.files
            .get()
            .ok_or_else(|| Failure::new(FileFailure::ServiceUnavailable, "核心没有启用文件服务"))
    }

    /// 文件访问范围里的这个调用方：令牌认出的 Bot 用它的 Matrix 用户名（和 `grants` 查到的一样）；没有 Bot 的（比如
    /// 不进聊天的工具插件）用插件 ID，它有点，用户名里没有，不会重名。
    fn bot_name(&self, caller: &Caller) -> Result<String, Failure> {
        let Some(bot) = &caller.bot else {
            return Ok(caller.id.to_string());
        };
        let localpart = self
            .plugins
            .bot_localpart(&caller.id, &bot.bot)
            .map_err(|e| Failure::new(FileFailure::Internal, e.to_string()))?;
        Ok(localpart.unwrap_or_else(|| caller.id.to_string()))
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

    /// 接手申请在冷却中：返回还要等多久。过期的记录顺手清掉。
    fn adopt_cooldown(&self, caller: &PluginId, id: &PluginId) -> Option<Duration> {
        let mut refusals = self.adopt_refusals.lock().expect("接手冷却表锁中毒");
        let key = (caller.clone(), id.clone());
        let now = Instant::now();
        let until = *refusals.get(&key)?;
        if until <= now {
            refusals.remove(&key);
            return None;
        }
        Some(until - now)
    }

    /// 记一次接手申请：冷却期内同一个 Bot 对同一个插件不再发卡，表里过期的顺手清掉。
    fn note_adopt_attempt(&self, caller: &PluginId, id: &PluginId) {
        let mut refusals = self.adopt_refusals.lock().expect("接手冷却表锁中毒");
        refusals.retain(|_, until| *until > Instant::now());
        refusals.insert(
            (caller.clone(), id.clone()),
            Instant::now() + ADOPT_COOLDOWN,
        );
    }

    /// 主人最近一次请求用的语言：Agent 和 Bot 的调用没有请求头，文字按它写（ADR-0043）。
    pub fn owner_languages(&self) -> Languages {
        self.plugins.owner_languages()
    }

    /// 卡片文字用谁的语言：发给家人的按他最近一次请求的 `Accept-Language`（ADR-0043）；他还没打开过 App、
    /// 没有记录时按主人的语言（协商空列表会落到英文，不是主人的语言）。主人的卡片按主人的。
    pub fn languages_for(&self, recipient: Option<&str>) -> Languages {
        match recipient.and_then(|person| self.plugins.person_languages(person)) {
            Some(header) => Languages::from_header(Some(&header)),
            None => self.owner_languages(),
        }
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
        // rmcp 按会话上的消息算空闲（默认 300 秒），等确认时没有消息，调用会被切断，Agent 拿不到结果。
        let mut sessions = LocalSessionManager::default();
        sessions.session_config.keep_alive =
            Some(self.confirmations.ttl() + SESSION_IDLE_AFTER_CARD);
        StreamableHttpService::new(
            move || Ok(server::Handler::new(bus.clone())),
            sessions.into(),
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
                bus.end_stopped_takeovers();
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
        if self.remote_contact_blocked(caller) {
            return contacts_gate::tools();
        }
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
        let activity = self.plugins.activity.start_tool(&caller.id);
        let name = request.name.to_string();
        let arguments = request.arguments.unwrap_or_default();
        let args = Value::Object(arguments.clone());
        let Some((owner, tool)) = naming::split(&name) else {
            return self.deny(
                caller,
                &name,
                &args,
                &ToolFailure::new("invalid_input", format!("没有工具 {name}")),
            );
        };
        if let Some(failure) = self.remote_contact_gate(caller, owner, tool) {
            return self.deny(caller, &name, &args, &failure);
        }
        if owner == naming::CORE {
            return core_tools::call(self, caller, tool, arguments, activity).await;
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
                &ToolFailure::new("not_found", format!("没有在运行的插件提供 {name}")),
            );
        };
        self.call_plugin(
            caller,
            &server,
            (&name, tool, activity),
            arguments,
            cancelled,
        )
        .await
    }

    /// 插件工具的声明：清单这会儿取不到、或插件没有这个工具时，给 Agent 的失败。
    /// 错误装箱：serde_json 开 `preserve_order` 时 `CallToolResult` 撑过 clippy `result_large_err` 的 128 字节阈值。
    async fn definition_of(
        &self,
        caller: &Caller,
        name: &str,
        args: &Value,
        server: &McpServer,
        tool: &str,
    ) -> Result<rmcp::model::Tool, Box<CallToolResult>> {
        let listed = match self.upstreams.tools(server).await {
            Ok(tools) => tools,
            // 插件的工具清单这会儿取不到：多半是插件没就绪或连接断了，Agent 该晚点再试。
            Err(e) => {
                return Err(Box::new(self.deny(
                    caller,
                    name,
                    args,
                    &ToolFailure::new("source_unavailable", e),
                )));
            }
        };
        listed.into_iter().find(|t| t.name == tool).ok_or_else(|| {
            Box::new(self.deny(
                caller,
                name,
                args,
                &ToolFailure::new("not_found", format!("插件 {} 没有工具 {tool}", server.id)),
            ))
        })
    }

    async fn call_plugin(
        self: &Arc<Self>,
        caller: &Caller,
        server: &McpServer,
        (name, tool, activity): (&str, &str, Guard),
        arguments: JsonObject,
        cancelled: impl Future<Output = ()> + Send,
    ) -> CallToolResult {
        let args = Value::Object(arguments.clone());
        let definition = match self.definition_of(caller, name, &args, server, tool).await {
            Ok(definition) => definition,
            Err(result) => return *result,
        };
        let read_only = definition
            .annotations
            .as_ref()
            .and_then(|a| a.read_only_hint)
            == Some(true);
        let highest = self.plugins.requires_root_confirmation(&server.id);
        let every_time = highest || risky::deletes(&definition);
        let policy = self.plugins.tool_policy();
        let clearance = || policy.clearance(caller.id.as_str(), server.id.as_str(), tool);
        let not_sensitive = if every_time {
            None
        } else if read_only {
            Some(Basis::PluginReadOnly)
        } else {
            clearance()
                .filter(|c| *c == Clearance::OwnerReadOnly)
                .map(Basis::from)
        };
        let mut cancelled = std::pin::pin!(cancelled);
        let (basis, held_ms, activity) = if not_sensitive.is_some() {
            (not_sensitive, None, activity)
        } else {
            drop(activity);
            let held_ms = match self
                .hold(caller, name, &args, &server.id, cancelled.as_mut())
                .await
            {
                Ok(held_ms) => held_ms,
                Err(result) => return *result,
            };
            // 停住期间主人可能改了放宽，时段也可能到期，所以放在停住之后判定。每次都问的工具全部不放宽。
            let cleared = if every_time {
                None
            } else {
                clearance().map(Basis::from).or_else(|| {
                    self.confirmations
                        .allow_in_window(&caller.id, server.id.as_str())
                        .map(Basis::Window)
                })
            };
            let activity = self.plugins.activity.start_tool(&caller.id);
            (cleared, held_ms, activity)
        };
        if let Some(basis) = basis {
            let outcome = forward(self, caller, server, tool, arguments, cancelled).await;
            self.record(
                caller,
                name,
                &args,
                Decision::Allowed,
                Some(Grounds {
                    basis: basis.clone(),
                    held_ms,
                }),
                &outcome,
            );
            // 放行时段里没发卡片，但这段是主人先前批准的：照实告诉 Agent。
            return match basis {
                Basis::Window(_) => with_confirmation(outcome.result, "window"),
                _ => outcome.result,
            };
        }
        let draft = self.tool_card(caller, server, &definition, &args, highest);
        // 不传 `cancelled`：确认过的调用不跟着 Agent 走，它断开了照样执行，结果在卡片上（ADR-0016）。
        let (agent, server, tool) = (caller.clone(), server.clone(), tool.to_owned());
        let run = move |bus: Arc<Bus>,
                        _secrets: SecretValues,
                        _settings: wonton_protocol::SettingValues| async move {
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
        self.confirmed_after_hold(
            caller,
            name,
            args,
            (draft, activity),
            run,
            (held_ms, Subject::Tool),
        )
        .await
    }

    /// 主人没同意：拒绝了、过期了，或者卡片发不出去。
    fn refused(
        &self,
        caller: &Caller,
        name: &str,
        args: &Value,
        refusal: &confirm::Refusal,
        held_ms: Option<u64>,
    ) -> CallToolResult {
        let decision = match refusal {
            confirm::Refusal::Rejected => Decision::Rejected,
            confirm::Refusal::Expired => Decision::Expired,
            _ => Decision::Unconfirmed,
        };
        let message = refusal.message();
        self.audit.record(&Record {
            caller: caller.id.as_str(),
            tool: name,
            arguments: args,
            decision,
            basis: Some(Grounds {
                basis: Basis::Card,
                held_ms,
            }),
            outcome: Some(&message),
            files: &[],
        });
        with_failure(&message, refusal.kind())
    }

    /// 总线拒绝：工具不在、参数不对、不让这个 Bot 做的事。类别进结果的 `_meta`，Agent 不用读文案。
    fn deny(
        &self,
        caller: &Caller,
        name: &str,
        args: &Value,
        failure: &ToolFailure,
    ) -> CallToolResult {
        self.audit.record(&Record {
            caller: caller.id.as_str(),
            tool: name,
            arguments: args,
            decision: Decision::Denied,
            basis: None,
            outcome: Some(&failure.message),
            files: &[],
        });
        with_failure(&failure.message, failure.kind)
    }

    fn record(
        &self,
        caller: &Caller,
        name: &str,
        args: &Value,
        decision: Decision,
        basis: Option<Grounds>,
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
            basis,
            outcome: Some(outcome),
            files,
        });
    }
}

#[cfg(test)]
mod activity_tests;
#[cfg(test)]
mod agent_install_adopt_tests;
#[cfg(test)]
mod agent_install_card_tests;
#[cfg(test)]
mod agent_install_isolation_tests;
#[cfg(test)]
mod agent_install_tests;
#[cfg(test)]
mod agent_permission_tests;
#[cfg(test)]
mod authoring_tests;
#[cfg(test)]
mod capability_install_tests;
#[cfg(test)]
mod contacts_gate_tests;
#[cfg(test)]
mod file_block_tests;
#[cfg(test)]
mod file_tests;
#[cfg(test)]
mod help_tests;
#[cfg(test)]
mod highest_tool_tests;
#[cfg(test)]
mod optional_permission_tests;
#[cfg(test)]
mod panel_tests;
#[cfg(test)]
mod people_tests;
#[cfg(test)]
mod profile_tests;
#[cfg(test)]
mod risky_tests;
#[cfg(test)]
mod save_tests;
#[cfg(test)]
mod takeover_tests;
#[cfg(test)]
mod test_setup;
#[cfg(test)]
pub(crate) mod test_support;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tool_failure_tests;
#[cfg(test)]
mod tool_policy_tests;
#[cfg(test)]
mod upstream_headers_tests;
#[cfg(test)]
mod upstream_pool_tests;
#[cfg(test)]
mod upstream_support;
#[cfg(test)]
mod upstream_tests;
#[cfg(test)]
mod window_tests;
