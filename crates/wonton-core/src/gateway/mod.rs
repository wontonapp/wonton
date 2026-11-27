//! 核心网关：按 Host 头分发请求；插件网页鉴权，登记的 OIDC 回调表单由应用校验（ADR-0078）。
//!
//! | Host | 去向 |
//! | --- | --- |
//! | `<base>` | 核心：登录、状态页、插件 API、OIDC 登录与授权确认端点 |
//! | `auth.<base>`、`auth.wonton.internal` | 身份提供方（Ory Hydra）的公开端点，发现文档按谁访问改写（ADR-0063） |
//! | `matrix.<base>` | Matrix 客户端接口，只给经隧道的已配对设备（`matrix`） |
//! | `push.wonton.internal` | 推送代理，只给 Tuwunel（`push`） |
//! | `notify.wonton.internal` | 插件发通知进"Wonton"私聊，只放行申请了 `notify` 的插件（`notify`） |
//! | `<slug>.<base>` | 插件网页，需要插件会话；本插件容器自访及受限的 OIDC 回调表单例外 |
//!
//! 经隧道接入的已配对设备不需要 Cookie：隧道入口（`crate::tunnel`）把设备身份放进请求扩展
//! （[`TunnelPeer`]），网关按设备登记表认出它的人（[`Person`]，ADR-0045）。局域网浏览器仍走核心会话加
//! 插件会话交接。本地代理可能被同机其他 App 借用，所以设备一侧的本地代理要校验会话凭据（ADR-0004、#20）；
//! 网关只在插件网页、OIDC 登录和授权确认端点、Matrix 登录令牌、Matrix 客户端接口、文件接口和插件管理接口（不能是网页
//! 发起的，ADR-0017）上接受隧道身份，其余管理 API 仍要主人令牌。
//!
//! 插件子域名彼此同站，也和核心同站：插件网页能让浏览器带着 Cookie 或隧道身份向别处发请求。浏览器里的
//! 隔离规则见 `isolation`：插件上只认同源请求和导航，核心上改状态的请求只接受同源，插件的 Cookie 出不了
//! 自己的子域名。

pub mod agent_chains;
mod agent_chains_api;
pub mod agent_outbox;
mod agent_permission;
mod api;
mod api_body;
mod api_switch;
mod auth;
mod authorization;
mod bot_activity;
mod bot_contacts;
mod bot_plugin_access;
mod bot_profile;
mod bot_projection;
mod bot_visibility;
mod bot_web;
#[cfg(test)]
mod bot_web_tests;
mod bots;
mod build;
mod capabilities;
mod capability_settings;
mod chain_mentions;
mod clients_api;
mod clients_domain;
mod clients_entry;
mod clients_passwords;
mod clients_status;
mod clients_switches;
mod confirmations;
mod core_routes;
mod core_upgrade;
mod devices;
mod entry;
mod entry_federation;
pub mod family;
mod family_private;
mod family_room;
mod files;
mod friend_gate;
mod friend_requests;
mod friends;
mod friends_wiring;
mod health;
mod hosted;
mod identity;
mod inbox;
mod install_gate_cards;
mod install_gate_checks;
mod install_gates;
mod invite_filter_guard;
mod isolation;
mod join_requests;
mod lan;
mod link_preview;
pub mod matrix;
mod matrix_rtc;
pub mod matrix_rtc_alias;
mod mcp;
mod mcp_apps;
mod me;
mod media;
mod model_settings;
mod models;
pub(crate) mod notify;
pub(crate) mod notify_limit;
mod onboarding;
mod owner_secrets;
mod pages;
mod peers;
pub mod people_limits;
pub(crate) mod permissions;
mod plugin_audience;
mod plugin_scope;
mod plugin_settings;
mod plugin_start;
mod plugin_web;
mod private_bot;
mod proxy;
mod proxy_sso;
mod push;
mod pushers;
mod registry_mirror;
mod request_guards;
mod roles;
mod room_rules;
pub(crate) mod send_limits;
pub mod sharing;
mod web_storage;

pub use entry::router as entry_router;
pub use push::router as push_router;
pub(crate) use request_guards::{same_origin_path, with_cookie};
pub(crate) use send_limits::Identity;
pub use send_limits::{SendLimiter, SendLimits};
mod relays_api;
mod session;
mod settings_api;
mod shared_bots;
#[cfg(test)]
#[path = "shared_bots_tests.rs"]
mod shared_bots_tests;
mod sso;
mod store_api;
mod tool_policy;

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use axum::Router;
use axum::extract::{ConnectInfo, Request};
use axum::response::Response;
use hyper::StatusCode;
use hyper::header::{HOST, SET_COOKIE};
use iroh::EndpointId;
use tower::ServiceExt;
use wonton_protocol::Role;

/// 家人的记录找不到了（比如刚被移除）：代登录和 OIDC 声明拿不到身份，如实报错。
#[derive(Debug, thiserror::Error)]
#[error("家人 {0} 不在记录里（可能刚被移除）")]
pub(crate) struct MissingMember(pub(crate) String);

use crate::config::{HostKind, PublicUrls};
use crate::friends::Friends;
use crate::http::HttpClient;
use crate::l10n;
use crate::matrix::{FederationBlocks, Matrix};
use crate::members::Members;
use crate::oidc::Hydra;
use crate::owner::OwnerStore;
use crate::plugin::PluginManager;
use crate::push::Push;
use crate::roles::Roles;
use crate::tunnel::Tunnel;

pub use pages::ERROR_HEADER;
pub use session::{APP_COOKIE, CORE_COOKIE};

/// 经隧道接入的设备：QUIC 握手认证过的 endpoint ID。只有核心的隧道入口会把它放进请求扩展，
/// 局域网 TCP 请求里没有，客户端也没法用请求头伪造。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TunnelPeer(pub EndpointId);

/// 网关转给插件网页时的“打开者”，值是 Matrix 用户 ID（ADR-0045 第 6 节）。插件网页拿它当说话人：
/// 上游自己的会话认不出走进来的是谁。只由网关写，客户端带的一律剥掉；插件把端口曝到宿主上时
/// 绕过网关的直连可以伪造。方向与 `X-Wonton-On-Behalf-Of` 相反（那个是插件发给核心）。
pub(super) const WEB_USER_HEADER: &str = "X-Wonton-Web-User";

/// 本地用户的 Matrix 用户 ID。配置里还没有 Matrix 时按公开域名拼：`public.base_domain` 和 Matrix 的
/// `server_name` 同出一个配置项，拼出来的和 `Matrix::user_id` 同形。
pub(super) fn matrix_user_id(gateway: &Gateway, username: &str) -> String {
    match &gateway.matrix {
        Some(matrix) => matrix.user_id(username),
        None => format!("@{username}:{}", gateway.public.base_domain),
    }
}

/// 网关认出的人（ADR-0045）：经隧道的已配对设备看设备记录里的人，局域网浏览器的核心会话是主人。
/// 认人只看隧道身份对应的设备记录，不读请求里的任何字段。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Person {
    pub username: String,
    pub role: Role,
}

impl Person {
    pub fn owner(username: &str) -> Self {
        Self {
            username: username.to_owned(),
            role: Role::Owner,
        }
    }

    pub fn member(username: &str) -> Self {
        Self {
            username: username.to_owned(),
            role: Role::Member,
        }
    }

    pub fn is_owner(&self) -> bool {
        self.role == Role::Owner
    }
}

pub struct Gateway {
    pub public: PublicUrls,
    pub owner: Arc<OwnerStore>,
    owner_token: String,
    pub members: Arc<Members>,
    family_room_sync: tokio::sync::Mutex<()>,
    member_sync: tokio::sync::Mutex<()>,
    /// 角色记录（ADR-0088）：主人以外的人按各自角色判断；主人短路为全部权限。
    pub roles: Arc<Roles>,
    /// 客户端入口的开关状态和连接守卫（ADR-0021，[`crate::clients::state`]）。
    pub(crate) clients: Arc<crate::clients::state::State>,
    pub plugins: Arc<PluginManager>,
    pub hydra: Option<Hydra>,
    /// 身份提供方公开端点的内部地址（`host:port`），`auth.<base>` 的请求转发到这里。
    pub hydra_public: Option<String>,
    pub http: HttpClient,
    system_http: HttpClient,
    /// 转发插件流量的池，空闲期比上游的 keep-alive 短（[`crate::http::FORWARD_POOL_IDLE`]）。
    upstream_http: HttpClient,
    pub tunnel: Tunnel,
    /// Matrix 服务器；没接时 App 的 Matrix 入口如实报不可用。
    pub matrix: Option<Arc<Matrix>>,
    /// Bot 发消息的频率和跳数（ADR-0045）。计数在内存里，核心重启从头算。
    pub(crate) send_limits: Arc<send_limits::SendLimiter>,
    /// 本机每个人发邀请、发消息的频率（ADR-0051）。计数只在内存里，重启从头算。
    pub(crate) people_limits: Arc<people_limits::PeopleLimiter>,
    /// 推送代理；没启用时登记和推送都如实报不可用。
    pub push: Option<Arc<Push>>,
    /// MCP 总线；没启用时 `mcp.wonton.internal` 如实报不可用。
    pub mcp: Option<crate::mcp::Endpoint>,
    /// 通知房间；没接 Matrix 时插件发通知如实报不可用。
    pub notices: Option<Arc<crate::matrix::NoticeRoom>>,
    /// 插件通知的频率限制（ADR-0047），计数只在内存里。
    pub(crate) notify_limiter: notify_limit::Limiter,
    /// 文件服务；没启用时文件接口如实报不可用。
    pub files: Option<Arc<crate::files::Files>>,
    /// 媒体服务（ADR-0113）；没接时媒体接口如实报不可用。
    pub media: Option<Arc<crate::media::Media>>,
    /// 审计日志（`core/audit.jsonl`，见 [`crate::mcp::audit`]）：总线和文件服务共用一份，
    /// 主人查看插件密钥的取值审计也从这里写。
    pub(crate) audit: Option<Arc<crate::mcp::audit::Audit>>,
    pub tusd: Option<String>,
    pub upgrades: Option<Arc<crate::upgrade::service::Upgrades>>,
    pub peers: Option<Arc<crate::peers::Peers>>,
    pub(crate) friends: Option<Arc<Friends>>,
    /// 公网上外人交的加好友请求（ADR-0088 第 98 行），只存内存。
    pub(crate) friend_requests: friend_requests::Requests,
    /// Bot 的联系人（ADR-0045、ADR-0050）；没接 Matrix 的核心没有。
    contacts: Option<Arc<crate::contacts::Contacts>>,
    pub(crate) federation_blocks: Option<Arc<FederationBlocks>>,
    pub sharing: Option<Arc<sharing::Sharing>>,
    pub(crate) onboarding: Arc<onboarding::OnboardingState>,
    health_limits: health::RateLimits,
    pub(crate) link_previews: Arc<crate::link_preview::LinkPreviews>,
    pub(crate) app_calls: Arc<mcp_apps::AppCalls>,
    /// 自己的域名和证书（ADR-0089）；核心没跑在装好的容器里时是空。
    pub(crate) domain: Option<Arc<crate::clients::domain::Domain>>,
    /// 客户端开关写入和入口配置刷新串行化，避免并发请求互相覆盖。
    pub(crate) clients_apply: tokio::sync::Mutex<()>,
    /// 启动流程创建的入口容器和公网访问总开关（ADR-0090）；客户端开关使用同一个实例应用模型，没装入口的核心没有。
    pub(crate) entry: Option<Arc<crate::entry_access::EntryAdmin>>,
    /// 公网访问总开关，默认关闭；插件开关保留但模型不生成公网标签。
    pub(crate) public_access: Arc<AtomicBool>,
    /// C12 入口种类配置完成与否；C7 只读取，连接器由后续 PR 接入。
    pub(crate) public_ingress_configured: bool,
    sessions: session::SessionStore,
    /// 调用链（ADR-0061 第 10 节）：账本、出箱和设置，见 [`agent_chains::AgentChains`]。
    pub(crate) agent_chains: Option<Arc<agent_chains::AgentChains>>,
}

/// 网关的完整路由。服务时需要 `into_make_service_with_connect_info::<SocketAddr>()`。
pub fn router(gateway: Arc<Gateway>) -> Router {
    let core = core_routes::routes(gateway.clone());
    Router::new().fallback(move |req: Request| dispatch(gateway.clone(), core.clone(), req))
}

fn request_host(req: &Request) -> String {
    req.headers()
        .get(HOST)
        .and_then(|h| h.to_str().ok())
        .map(str::to_owned)
        .or_else(|| req.uri().authority().map(|a| a.to_string()))
        .unwrap_or_default()
}

async fn dispatch(gateway: Arc<Gateway>, core: Router, req: Request) -> Response {
    if req.extensions().get::<sharing::SharePeer>().is_some() {
        return sharing::dispatch(gateway, core, req).await;
    }
    let host = request_host(&req);
    match gateway.public.classify(&host) {
        HostKind::Core => match core.oneshot(req).await {
            Ok(response) => response,
            Err(never) => match never {},
        },
        HostKind::Auth => auth::request(&gateway, req).await,
        HostKind::Matrix => matrix::client_request(gateway, req).await,
        HostKind::PluginMatrix => matrix::bot_request(&gateway, req).await,
        HostKind::PluginModels => models::request(&gateway, req).await,
        HostKind::PluginCapability(capability) => {
            capabilities::request(&gateway, &capability, req).await
        }
        HostKind::Push => push::gateway_request(&gateway, req).await,
        HostKind::PluginMcp => mcp::request(&gateway, req).await,
        HostKind::PluginHealth => health::request(&gateway, req).await,
        HostKind::PluginNotify => notify::request(&gateway, req).await,
        HostKind::FileHooks => files::hook_request(&gateway, req).await,
        HostKind::Plugin(slug) => plugin_web::request(&gateway, &slug, req).await,
        HostKind::Lan(ip) => lan::request(&gateway, ip, &req),
        HostKind::Unknown => {
            let langs = api::languages(req.headers());
            pages::error(
                StatusCode::NOT_FOUND,
                "unknown_host",
                &l10n::text(
                    &langs,
                    "gateway-unknown-host",
                    &[("host", host.as_str().into())],
                ),
                &langs,
                pages::Format::of(req.headers()),
            )
        }
    }
}

fn client_addr(req: &Request) -> Option<SocketAddr> {
    req.extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ConnectInfo(addr)| *addr)
}

/// TCP 来源是不是这个内部服务：地址现查，容器重建后会变。服务没配置时谁都不是。
async fn from_service(authority: Option<&str>, peer: Option<SocketAddr>, name: &str) -> bool {
    let (Some(authority), Some(peer)) = (authority, peer) else {
        return false;
    };
    match tokio::net::lookup_host(authority).await {
        Ok(mut addrs) => addrs.any(|addr| addr.ip().to_canonical() == peer.ip().to_canonical()),
        Err(e) => {
            tracing::warn!(service = name, error = %e, "查不到内部服务的地址，拒绝它的请求");
            false
        }
    }
}

#[cfg(test)]
mod agent_chains_test_support;
#[cfg(test)]
mod agent_chains_tests;
#[cfg(test)]
mod bot_activity_tests;
#[cfg(test)]
mod bot_contacts_tests;
#[cfg(test)]
mod bot_profile_tests;
#[cfg(test)]
mod bots_tests;
#[cfg(test)]
mod capabilities_tests;
#[cfg(test)]
mod capability_uninstall_tests;
#[cfg(test)]
mod chain_mentions_tests;
// 原有联邦用例照旧跑（main 上这个文件一度没人挂载，用例没在跑）：421、点段和发现文件的行为由它们锁住。
#[cfg(test)]
#[path = "clients_entry_tests.rs"]
mod clients_entry_tests;
#[cfg(test)]
mod clients_status_tests;
#[cfg(test)]
mod core_upgrade_tests;
#[cfg(test)]
mod degraded_webui_tests;
#[cfg(test)]
mod egress_api_tests;
#[cfg(test)]
#[path = "entry_tests.rs"]
mod entry_tests;
#[cfg(test)]
pub(crate) mod fakes;
#[cfg(test)]
mod family_plugin_tests;
#[cfg(test)]
mod family_roles_tests;
#[cfg(test)]
mod family_rules_tests;
#[cfg(test)]
mod family_tests;
#[cfg(test)]
mod files_create_tests;
#[cfg(test)]
mod files_plugin_tests;
#[cfg(test)]
mod files_revoke_tests;
#[cfg(test)]
mod files_tests;
#[cfg(test)]
mod files_upload_failure_tests;
#[cfg(test)]
mod files_upload_tests;
#[cfg(test)]
mod first_run_tests;
#[cfg(test)]
mod friend_gate_tests;
#[cfg(test)]
mod friends_tests;
#[cfg(test)]
mod health_tests;
#[cfg(test)]
mod inbox_tests;
#[cfg(test)]
mod install_gate_tests;
#[cfg(test)]
mod isolation_policy_tests;
#[cfg(test)]
mod isolation_tests;
#[cfg(test)]
mod join_recovery_tests;
#[cfg(test)]
mod join_reissue_tests;
#[cfg(test)]
mod join_requests_tests;
#[cfg(test)]
mod join_role_tests;
#[cfg(test)]
mod l10n_tests;
#[cfg(test)]
mod lan_api_tests;
#[cfg(test)]
mod lan_tests;
#[cfg(test)]
mod limits_tests;
#[cfg(test)]
mod local_friends_tests;
#[cfg(test)]
mod matrix_invite_filter_tests;
#[cfg(test)]
mod matrix_rtc_docker_tests;
#[cfg(test)]
mod matrix_rtc_tests;
#[cfg(test)]
mod matrix_sync_restart_tests;
#[cfg(test)]
mod mcp_takeover_tests;
#[cfg(test)]
mod mcp_tests;
#[cfg(test)]
mod media_browse_tests;
#[cfg(test)]
mod media_command_tests;
#[cfg(test)]
mod media_flow_tests;
#[cfg(test)]
mod media_quota_tests;
#[cfg(test)]
mod media_stream_tests;
#[cfg(test)]
mod media_tests;
#[cfg(test)]
mod models_tests;
#[cfg(test)]
mod notify_tests;
#[cfg(test)]
mod oidc_callback_tests;
#[cfg(test)]
mod oidc_consent_tests;
#[cfg(test)]
mod onboarding_tests;
#[cfg(test)]
mod owner_secrets_tests;
#[cfg(test)]
mod pairing_tests;
#[cfg(test)]
mod peers_api_tests;
#[cfg(test)]
mod plugin_capability_tests;
#[cfg(test)]
mod plugin_forms_tests;
#[cfg(test)]
mod plugin_install_failure_tests;
#[cfg(test)]
mod plugin_start_tests;
#[cfg(test)]
mod proxy_sso_tests;
#[cfg(test)]
mod push_tests;
#[cfg(test)]
mod registry_mirror_tests;
#[cfg(test)]
mod relays_api_tests;
#[cfg(test)]
mod secret_change_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tunnel_bot_tests;
#[cfg(test)]
mod tunnel_tests;
#[cfg(test)]
mod tusd_tests;
#[cfg(test)]
mod upload_settings_tests;
#[cfg(test)]
mod web_user_tests;
