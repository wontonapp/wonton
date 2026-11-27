//! MCP 总线（ADR-0006、ADR-0016）：Agent 经 `mcp.wonton.internal` 连上来，看到核心自己的工具和所有在运行的
//! 插件声明的工具。调用方由网关按来源地址认出（[`Caller`]）；没标只读、主人也没放宽（ADR-0024）的工具先请主人确认
//! （[`confirm`]）；每次调用写一行审计（[`audit`]）。

mod agent_cards;
mod agent_install;
mod agent_permission;
mod apps;
pub(crate) mod audience;
pub mod audit;
mod authoring_tools;
mod bot_access;
mod bounded_http;
#[path = "bus/build.rs"]
mod build;
mod cards;
pub(crate) mod chain_card;
mod confirm;
mod confirmed;
mod contacts_gate;
mod core_tools;
mod dependencies;
#[path = "bus/deps.rs"]
mod deps;
mod file_results;
mod file_tools;
#[path = "bus/identity.rs"]
mod identity;
mod naming;
mod notifier;
mod people_tools;
mod profile_tools;
mod risky;
mod save_tools;
mod send_message_tool;
mod server;
#[path = "bus/session.rs"]
mod session;
mod takeover;
mod tool_policy;
#[path = "bus/tools.rs"]
mod tools;
pub mod upstream;
mod windows;

use std::collections::HashMap;
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
pub(crate) use cards::{highest_confirm_for, mark_nvme_risk};
pub use confirm::{AnswerError, Audience, Confirmations, Notifier, Recipients, Refusal, Ticket};
pub(crate) use dependencies::{disclose_connections, disclose_provider};
pub use notifier::DevicePush;
pub use server::{Caller, TurnSender};
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
    /// 收件箱（ADR-0119）。没接时收件箱工具和接口如实报不可用。
    inbox: OnceLock<Arc<crate::inbox::Inbox>>,
    /// 旧协议（2026-07-28 之前）的会话：工具列表变了要逐个通知。
    peers: Mutex<Vec<Peer<RoleServer>>>,
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
mod clients_passwords_isolation_tests;
#[cfg(test)]
mod contacts_gate_tests;
#[cfg(test)]
mod device_card_tests;
#[cfg(test)]
mod file_block_tests;
#[cfg(test)]
mod file_tests;
#[cfg(test)]
mod help_tests;
#[cfg(test)]
mod highest_tool_tests;
#[cfg(test)]
mod inbox_tools_tests;
#[cfg(test)]
mod install_gate_tests;
#[cfg(test)]
mod missing_sender_tests;
#[cfg(test)]
mod optional_permission_tests;
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
