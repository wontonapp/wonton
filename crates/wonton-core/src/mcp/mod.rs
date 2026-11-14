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
mod bot_access;
mod bounded_http;
mod cards;
pub(crate) mod chain_card;
mod confirm;
mod confirmed;
mod contacts_gate;
mod core_tools;
mod dependencies;
mod file_results;
mod file_tools;
mod handoff;
mod handoff_tools;
mod naming;
mod notifier;
mod panel_tools;
mod people_tools;
mod profile_tools;
mod risky;
mod save_tools;
mod send_message_tool;
mod server;
mod takeover;
mod tool_policy;
pub mod upstream;
mod windows;

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use rmcp::model::{CallToolRequestParams, CallToolResult, JsonObject, Tool};
use rmcp::service::{Peer, RoleServer};
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use serde_json::Value;
use wonton_protocol::{AiLevel, ConfirmationState, FileFailure, SecretValues};

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
pub use confirm::{AnswerError, Audience, Confirmations, Notifier, Recipients, Refusal, Ticket};
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
    pub(crate) server_name: String,
    pub(crate) members: Arc<crate::members::Members>,
    pub(crate) roles: Arc<crate::roles::Roles>,
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
    /// 家人记录、角色记录和本服务器的 Matrix 服务器名：`wonton__user_locale` 认人、开卡挑人用（[`Bus::attach_people`]）。
    people: OnceLock<People>,
    /// Matrix 和主人：Bot 钉进主面板的同意卡片要发进 Bot 的房间里（[`Bus::attach_matrix`]）。
    matrix: OnceLock<Cards>,
    /// Bot 的联系人：有远端联系人的 Bot 只聊天（[`Bus::attach_contacts`]，ADR-0050）。
    contacts: OnceLock<Arc<crate::contacts::Contacts>>,
    /// 调用链（ADR-0061 第 10 节）：权限卡片按会话认到链上、停链时撤回，上限卡片也从这里发。
    agent_chains: OnceLock<Arc<crate::gateway::agent_chains::AgentChains>>,
    /// 好友名单（ADR-0051）：`wonton__send_message` 认收件人用（[`Bus::attach_friends`]）。
    friends: OnceLock<Arc<crate::friends::Friends>>,
    handoffs: Arc<handoff::Handoffs>,
    handoff_gateway: OnceLock<std::sync::Weak<crate::gateway::Gateway>>,
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
            agent_chains: OnceLock::new(),
            friends: OnceLock::new(),
            handoffs: Arc::default(),
            handoff_gateway: OnceLock::new(),
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

    /// 接上家人记录、角色记录和本服务器的 Matrix 服务器名（没接 Matrix 的核心是基础域名）。没接上时
    /// `wonton__user_locale` 如实报错，开卡片按没人有装卸权限算。
    pub fn attach_people(
        &self,
        server_name: String,
        members: Arc<crate::members::Members>,
        roles: Arc<crate::roles::Roles>,
    ) {
        if self
            .people
            .set(People {
                server_name,
                members,
                roles,
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

    /// 接上调用链的服务（ADR-0061 第 10 节）。只接一次。
    pub fn attach_agent_chains(&self, chains: Arc<crate::gateway::agent_chains::AgentChains>) {
        if self.agent_chains.set(chains).is_err() {
            tracing::warn!("调用链的服务接了两次，用先接上的");
        }
    }

    pub(crate) fn agent_chains(&self) -> Option<&Arc<crate::gateway::agent_chains::AgentChains>> {
        self.agent_chains.get()
    }

    pub fn attach_handoff_gateway(&self, gateway: std::sync::Weak<crate::gateway::Gateway>) {
        if self.handoff_gateway.set(gateway).is_err() {
            tracing::error!("交接的网关接了两次，用先接上的");
        }
    }

    /// 接上好友名单（ADR-0051）。只接一次。
    pub fn attach_friends(&self, friends: Arc<crate::friends::Friends>) {
        if self.friends.set(friends).is_err() {
            tracing::warn!("好友名单接了两次，用先接上的");
        }
    }

    pub(crate) fn friends(&self) -> Option<&Arc<crate::friends::Friends>> {
        self.friends.get()
    }

    pub fn audit(&self) -> &Arc<Audit> {
        &self.audit
    }

    /// 停整条链（ADR-0061 第 10 节）：账本置 `Stopped`、桥的轮次进他的出箱、`agent_acp` 的会话交给网关取消，
    /// 链上还等着的卡片撤回（等待方拿到 `cancelled`）。
    pub async fn stop_chain(
        self: &Arc<Self>,
        chain: &str,
    ) -> Result<crate::agent_chain::ChainStopOutcome, crate::agent_chain::ChainLedgerError> {
        use crate::agent_chain::ChainLedgerError;
        let Some(chains) = self.agent_chains.get() else {
            return Err(ChainLedgerError::Invalid("调用链的服务没接上".into()));
        };
        let outcome = chains.stop(chain)?;
        chain_card::withdraw_cards(self, &outcome);
        Ok(outcome)
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
    /// `tools/call` 转给插件时的调用者身份（ADR-0088 第 8 节）：私人 Bot 带归属人，主人和在这个
    /// 应用里当管理员（“能打开”且勾了“当管理员”）的人再带 [`Capability::PluginAdmin`]；共享 Bot
    /// 和认不出的人两个都不带，插件只能按全家共用的数据处理。认人的口径和工具放行一致（`bot_access`）：
    /// 记录在、人没被移除，就认。
    pub(crate) fn tool_call_identity(
        &self,
        caller: &Caller,
        plugin: &PluginId,
    ) -> upstream::CallIdentity {
        use wonton_protocol::{BotKeeper, Capability, MemberStatus};
        let Some(username) = caller.bot.as_ref().and_then(|bot| match &bot.keeper {
            BotKeeper::User { username } => Some(username),
            BotKeeper::Household | BotKeeper::Other => None,
        }) else {
            return upstream::CallIdentity::EMPTY;
        };
        if username == self.plugins.owner_name() {
            return upstream::CallIdentity {
                user: Some(username.clone()),
                capabilities: vec![Capability::PluginAdmin.as_str().to_owned()],
            };
        }
        let people = self.people.get();
        let Some(member) = people
            .and_then(|people| people.members.get(username))
            .filter(|member| member.status != MemberStatus::Removed)
        else {
            return upstream::CallIdentity::EMPTY;
        };
        let capabilities = people
            .and_then(|people| people.roles.get(&member.role_id))
            .filter(|role| {
                role.permissions.open_apps.allows_plugin(plugin.as_str())
                    && role.permissions.plugin_admin.allows_plugin(plugin.as_str())
            })
            .map(|_| vec![Capability::PluginAdmin.as_str().to_owned()])
            .unwrap_or_default();
        upstream::CallIdentity {
            user: Some(username.clone()),
            capabilities,
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

    /// 调用方能看到的工具：核心自己的，加上所有在运行、对它放行（[`Bus::bot_access`]）的插件的。并发地问，一个插件卡住
    /// 挡不住别的（每个插件有期限，见 [`Upstreams::tools`]）。列不出工具的插件记一条警告后跳过，它的工具这时本来也调不通。
    ///
    /// 封顶“只读”的（ADR-0088 第 5 节）只列只读的插件工具；核心安装等工具各按自己的权限项管，`save_file` 另按 AI 级别管。
    async fn tools(&self, caller: &Caller) -> Vec<Tool> {
        if self.remote_contact_blocked(caller) {
            return contacts_gate::tools();
        }
        let cap = self.ai_cap(caller);
        let mut servers = self.plugins.mcp_servers();
        servers.retain(|server| self.bot_access(caller, &server.id).is_ok());
        let listed = servers.iter().map(|server| async {
            match self.unconfigured(&server.id) {
                Some(notice) => {
                    self.upstreams.note_problem(server, notice.clone());
                    Err(notice)
                }
                None => self.upstreams.tools(server).await,
            }
        });
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
        if cap == AiLevel::ReadOnly {
            let save = naming::join(naming::CORE, save_tools::SAVE);
            tools.retain(|tool| {
                tool.name != save
                    && (naming::split(&tool.name).is_some_and(|(owner, _)| owner == naming::CORE)
                        || risky::classify(tool) == AiLevel::ReadOnly)
            });
        }
        tools
    }

    /// 插件必填的密钥还没填时的说明，填齐后为空：它的 MCP 工具这时连上也会被对方拒，调用直接按未就绪回绝并
    /// 点名缺哪项（ADR-0064）。
    fn unconfigured(&self, id: &PluginId) -> Option<String> {
        let missing = self.plugins.unfilled_secrets(id, None);
        (!missing.is_empty()).then(|| {
            format!(
                "插件 {id} 还没填好密钥，先填 {} 再用它的工具",
                missing.join("、")
            )
        })
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
        if let Err(failure) = self.bot_access(caller, &server.id) {
            return self.deny(caller, &name, &args, &failure);
        }
        if let Some(notice) = self.unconfigured(&server.id) {
            return self.deny(caller, &name, &args, &ToolFailure::new("not_ready", notice));
        }
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

    /// 一次插件工具调用的确认决定：封顶不够直接拒（失败即停），会删、会覆盖的调用在“删除要管理员批”的封顶下
    /// 把卡片交给管理员（ADR-0088 第 5 节），其余按 ADR-0024。
    fn plugin_decision(
        &self,
        caller: &Caller,
        server: &McpServer,
        definition: &Tool,
    ) -> Result<risky::ToolDecision, ToolFailure> {
        let cap = self.ai_cap(caller);
        let tool_level = risky::classify(definition);
        if risky::capped(cap, tool_level) {
            return Err(ToolFailure::new(
                "permission_denied",
                "这次的 AI 动手级别是只读，这个工具会改或删数据，没有执行",
            ));
        }
        let highest = self.plugins.requires_root_confirmation(&server.id);
        let subject = if cap == AiLevel::AdminApprovesDeletes && tool_level == AiLevel::SelfConfirms
        {
            Subject::ToolDelete
        } else {
            Subject::Tool
        };
        let relaxed = cap == AiLevel::SelfConfirms && handoff::allows_relaxation(self, caller);
        Ok(risky::ToolDecision::new(
            definition, cap, highest, subject, relaxed,
        ))
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
        let decision = match self.plugin_decision(caller, server, &definition) {
            Ok(decision) => decision,
            Err(failure) => return self.deny(caller, name, &args, &failure),
        };
        let policy = self.plugins.tool_policy();
        let clearance = || {
            decision
                .relaxed
                .then(|| policy.clearance(caller.id.as_str(), server.id.as_str(), tool))
                .flatten()
        };
        let not_sensitive = if decision.every_time {
            None
        } else if decision.read_only {
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
            let cleared = if decision.every_time || !decision.relaxed {
                None
            } else {
                clearance().map(Basis::from).or_else(|| {
                    self.confirmations
                        .allow_in_window(
                            &caller.id,
                            &risky::window_scope(caller, server.id.as_str()),
                        )
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
        let draft = self.tool_card(caller, server, &definition, &args, decision);
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
            (held_ms, decision.subject),
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
mod admin_cards_tests;
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
mod ai_level_tests;
#[cfg(test)]
mod authoring_tests;
#[cfg(test)]
mod bot_access_tests;
#[cfg(test)]
mod capability_install_tests;
#[cfg(test)]
mod contacts_gate_tests;
#[cfg(test)]
mod file_block_tests;
#[cfg(test)]
mod file_tests;
#[cfg(test)]
mod handoff_path_tests;
#[cfg(test)]
mod handoff_test_support;
#[cfg(test)]
mod handoff_tests;
#[cfg(test)]
mod help_tests;
#[cfg(test)]
mod highest_tool_tests;
#[cfg(test)]
mod install_gate_tests;
#[cfg(test)]
mod optional_permission_tests;
#[cfg(test)]
mod panel_tests;
#[cfg(test)]
mod people_tests;
#[cfg(test)]
mod profile_tests;
#[cfg(test)]
mod readable_card_tests;
#[cfg(test)]
mod risky_tests;
#[cfg(test)]
mod save_tests;
#[cfg(test)]
mod send_message_tests;
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
mod tool_identity_tests;
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
