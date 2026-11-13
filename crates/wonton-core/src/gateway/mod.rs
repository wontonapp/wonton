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

pub mod agent_acp;
pub mod agent_chains;
mod agent_chains_api;
pub mod agent_outbox;
mod agent_permission;
mod api;
mod api_body;
mod api_switch;
mod auth;
mod bot_activity;
mod bot_contacts;
mod bot_plugin_access;
mod bot_profile;
mod bot_web;
#[cfg(test)]
mod bot_web_tests;
mod bots;
mod capabilities;
mod capability_settings;
mod chain_mentions;
mod clients_api;
mod clients_domain;
mod clients_entry;
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
mod friends;
mod friends_wiring;
mod health;
mod hosted;
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
mod model_providers;
mod models;
pub(crate) mod notify;
pub(crate) mod notify_limit;
mod onboarding;
mod owner_secrets;
mod pages;
mod panel_pins;
mod peers;
pub mod people_limits;
pub(crate) mod permissions;
mod plugin_scope;
mod plugin_settings;
mod plugin_start;
mod plugin_web;
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
pub use send_limits::{SendLimiter, SendLimits};
mod session;
mod settings_api;
mod sso;
mod store_api;
mod tool_policy;

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use axum::extract::{ConnectInfo, Request};
use axum::response::Response;
use hyper::header::{ACCEPT_LANGUAGE, AUTHORIZATION, HOST, SET_COOKIE};
use hyper::{HeaderMap, StatusCode};
use iroh::EndpointId;
use tower::ServiceExt;
use wonton_protocol::{MemberStatus, Permissions, Role};

use crate::roles::RolesError;

/// 家人的记录找不到了（比如刚被移除）：代登录和 OIDC 声明拿不到身份，如实报错。
#[derive(Debug, thiserror::Error)]
#[error("家人 {0} 不在记录里（可能刚被移除）")]
pub(crate) struct MissingMember(pub(crate) String);

use crate::config::{self, HostKind, PublicUrls};
use crate::friends::Friends;
use crate::http::HttpClient;
use crate::l10n;
use crate::matrix::{FederationBlocks, Matrix};
use crate::members::Members;
use crate::oidc::Hydra;
use crate::owner::{Owner, OwnerStore};
use crate::plugin::PluginManager;
use crate::push::Push;
use crate::roles::Roles;
use crate::secret::constant_time_eq;
use crate::tunnel::Tunnel;

pub use pages::ERROR_HEADER;
pub use session::{APP_COOKIE, CORE_COOKIE};

/// 经隧道接入的设备：QUIC 握手认证过的 endpoint ID。只有核心的隧道入口会把它放进请求扩展，
/// 局域网 TCP 请求里没有，客户端也没法用请求头伪造。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TunnelPeer(pub EndpointId);

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
    /// 审计日志（`core/audit.jsonl`，见 [`crate::mcp::audit`]）：总线和文件服务共用一份，
    /// 主人查看插件密钥的取值审计也从这里写。
    pub(crate) audit: Option<Arc<crate::mcp::audit::Audit>>,
    pub tusd: Option<String>,
    pub upgrades: Option<Arc<crate::upgrade::service::Upgrades>>,
    pub peers: Option<Arc<crate::peers::Peers>>,
    pub(crate) friends: Option<Arc<Friends>>,
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
    sessions: session::SessionStore,
    /// 调用链（ADR-0061 第 10 节）：账本、出箱、设置和 `agent_acp` 的取消方，见 [`agent_chains::AgentChains`]。
    pub(crate) agent_chains: Option<Arc<agent_chains::AgentChains>>,
    /// `agent_acp` 的活连接注册表（ADR-0061 第 5 块）：并发计数和停链取消。
    pub(crate) agent_acp: Option<Arc<agent_acp::Registry>>,
}
impl Gateway {
    /// `hydra` 带上身份提供方公开端点的地址，必须来自 [`crate::config::Config`]：配置层已经校验过它是
    /// `http://主机:端口`。
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        public: PublicUrls,
        owner: Arc<OwnerStore>,
        owner_token: String,
        members: Arc<Members>,
        plugins: Arc<PluginManager>,
        hydra: Option<(Hydra, String)>,
        http: HttpClient,
        tunnel: Tunnel,
    ) -> Self {
        plugins.attach_person_members(members.clone());
        let (hydra, hydra_public) = match hydra {
            Some((hydra, public_url)) => {
                let authority = config::http_authority(&public_url)
                    .unwrap_or_else(|e| panic!("身份提供方的公开地址没有经过配置层校验：{e}"));
                (Some(hydra), Some(authority))
            }
            None => (None, None),
        };
        let onboarding = Arc::new(onboarding::OnboardingState::new(plugins.data_dir()));
        let link_previews = Arc::new(crate::link_preview::LinkPreviews::new(
            plugins.data_dir().join("core").join("link-previews"),
            plugins.core_proxy().clone(),
        ));
        let roles = Arc::new(Roles::initial());
        tunnel.pairing.attach_roles(roles.clone());
        plugins.attach_person_roles(roles.clone());
        Self {
            public,
            owner,
            owner_token,
            members,
            family_room_sync: tokio::sync::Mutex::new(()),
            member_sync: tokio::sync::Mutex::new(()),
            roles,
            clients: plugins.clients.clone(),
            plugins,
            hydra,
            hydra_public,
            http,
            system_http: HttpClient::system_loopback(),
            upstream_http: HttpClient::for_forwarding(),
            tunnel,
            matrix: None,
            send_limits: Arc::new(send_limits::SendLimiter::new(
                send_limits::SendLimits::default(),
            )),
            people_limits: Arc::new(people_limits::PeopleLimiter::new(
                people_limits::PeopleLimits::default(),
            )),
            push: None,
            mcp: None,
            notices: None,
            notify_limiter: notify_limit::Limiter::default(),
            files: None,
            audit: None,
            tusd: None,
            upgrades: None,
            peers: None,
            friends: None,
            contacts: None,
            federation_blocks: None,
            sharing: None,
            onboarding,
            health_limits: health::RateLimits::new(),
            link_previews,
            app_calls: Arc::default(),
            domain: None,
            sessions: session::SessionStore::default(),
            agent_chains: None,
            agent_acp: None,
        }
    }

    #[must_use]
    pub fn with_clients(mut self, domain: Option<Arc<crate::clients::domain::Domain>>) -> Self {
        self.domain = domain;
        self
    }

    #[must_use]
    pub fn with_sharing(mut self, sharing: Option<Arc<sharing::Sharing>>) -> Self {
        self.sharing = sharing;
        self
    }

    /// 接上角色记录：网关自己只带模板时用 [`Roles::initial`]，生产由启动时装上 `roles.json`。
    #[must_use]
    pub fn with_roles(mut self, roles: Arc<Roles>) -> Self {
        self.tunnel.pairing.attach_roles(roles.clone());
        self.plugins.attach_person_roles(roles.clone());
        self.roles = roles;
        self
    }

    #[must_use]
    pub fn with_system_http(mut self, http: HttpClient) -> Self {
        self.system_http = http;
        self
    }

    /// 接上 Matrix 服务器：App 的登录令牌和 `matrix.<base>` 这才可用。
    #[must_use]
    pub fn with_matrix(mut self, matrix: Option<Arc<Matrix>>) -> Self {
        self.matrix = matrix;
        self
    }

    /// 换上配置里的 Bot 发消息限制（[`crate::config::Config`] 的环境变量改数字）；测试可以带自己的时钟。
    #[must_use]
    pub fn with_send_limiter(mut self, send_limits: Arc<send_limits::SendLimiter>) -> Self {
        self.send_limits = send_limits;
        self
    }

    /// 接上推送代理：设备登记推送、Tuwunel 调 `push.wonton.internal` 这才可用。
    #[must_use]
    pub fn with_push(mut self, push: Option<Arc<Push>>) -> Self {
        self.push = push;
        self
    }

    /// 接上 MCP 总线：`mcp.wonton.internal` 和回答确认的接口这才可用。
    #[must_use]
    pub fn with_mcp(mut self, mcp: Option<crate::mcp::Endpoint>) -> Self {
        self.mcp = mcp;
        self
    }

    /// 接上通知房间：`notify.wonton.internal` 这才可用（ADR-0047）。
    #[must_use]
    pub fn with_notices(mut self, notices: Option<Arc<crate::matrix::NoticeRoom>>) -> Self {
        self.notices = notices;
        self
    }

    /// 接上文件服务：`/_wonton/files/…` 这才可用。
    #[must_use]
    pub fn with_files(mut self, files: Option<Arc<crate::files::Files>>) -> Self {
        self.files = files;
        self
    }

    /// 接上审计日志：主人查看插件密钥的取值审计这才可用。
    #[must_use]
    pub fn with_audit(mut self, audit: Option<Arc<crate::mcp::audit::Audit>>) -> Self {
        self.audit = audit;
        self
    }

    /// 接上 tusd：`authority` 是 `host:port`，见 [`crate::config::http_authority`]。
    pub fn with_tusd(mut self, authority: Option<String>) -> Self {
        self.tusd = authority;
        self
    }

    /// 接上核心的一键升级：`/_wonton/api/core/upgrade` 这才可用。
    #[must_use]
    pub fn with_upgrades(
        mut self,
        upgrades: Option<Arc<crate::upgrade::service::Upgrades>>,
    ) -> Self {
        self.upgrades = upgrades;
        self
    }

    /// 接上服务器配对：`/_wonton/api/peers` 和联邦转发这才可用（ADR-0050）。没接 Matrix 的核心没有它。
    #[must_use]
    pub fn with_peers(mut self, peers: Option<Arc<crate::peers::Peers>>) -> Self {
        self.peers = peers;
        self
    }

    fn check_owner_token(&self, token: &str) -> bool {
        constant_time_eq(token.trim(), &self.owner_token)
    }

    /// 访问令牌是谁（Matrix 用户 ID）：`whoami`，按令牌的哈希缓存（和发送限制共用一份，见
    /// [`send_limits::SendLimiter`]）。没接入 Matrix 或认不出时是 `None`。
    pub(crate) async fn bot_user_of_token(&self, token: &str) -> Option<String> {
        self.send_limits
            .user_id(
                token,
                send_limits::token_hash(token),
                self.matrix.as_deref(),
            )
            .await
    }

    /// 经隧道接入的已配对设备是谁（ADR-0045）：设备记录里写了家人就是他，没写是主人。吊销后立即不再认它，
    /// 哪怕请求还在一条没来得及断开的连接上。
    fn device_person(&self, peer: Option<TunnelPeer>) -> Option<Person> {
        let TunnelPeer(id) = peer?;
        let device = self.tunnel.devices.get(&id)?;
        let Some(username) = device.user else {
            return Some(Person::owner(self.owner.username()));
        };
        // 失败即停：设备记的家人必须在记录里、还没被移除。记录对不上时按认不出处理，不编一个身份放行。
        match self.members.get(&username) {
            Some(record)
                if record.status != MemberStatus::Removed
                    && !self.members.pending_endpoint(id, &username) =>
            {
                Some(Person::member(&record.username))
            }
            _ => {
                tracing::warn!(
                    device = %id.fmt_short(),
                    member = %username,
                    "设备的家人不在记录里、尚未批准或已移除，拒绝这台设备的请求"
                );
                None
            }
        }
    }

    /// 经隧道接入的已配对设备属于谁：用户名（家人）或主人。
    fn device_user(&self, peer: Option<TunnelPeer>) -> Option<String> {
        self.device_person(peer).map(|person| person.username)
    }

    /// 请求认得出是谁时，把它的 `Accept-Language` 记在这个人名下（ADR-0043）：语言按人给，主人和家人各记各的。
    /// 认不出人就不记。
    pub(crate) fn note_languages(&self, peer: Option<TunnelPeer>, headers: &HeaderMap) {
        let person = self.device_person(peer).or_else(|| {
            let token = headers
                .get(AUTHORIZATION)?
                .to_str()
                .ok()?
                .strip_prefix("Bearer ")?;
            self.check_owner_token(token)
                .then(|| Person::owner(self.owner.username()))
        });
        let Some(person) = person else { return };
        let accept = headers.get(ACCEPT_LANGUAGE).and_then(|v| v.to_str().ok());
        self.plugins.note_person_languages(&person.username, accept);
    }

    /// 核心域名上的当前人：经隧道的已配对设备，或者局域网浏览器的核心会话（主人令牌登录，是主人）。
    fn core_person(&self, peer: Option<TunnelPeer>, headers: &HeaderMap) -> Option<Person> {
        self.device_person(peer).or_else(|| {
            session::cookies(headers, CORE_COOKIE).find_map(|sid| self.sessions.core_user(sid))
        })
    }

    /// 插件域上的当前人：已配对设备或本域插件会话。来源隔离或受限 OIDC 回调允许后才使用。
    fn plugin_person(&self, host: &str, req: &Request) -> Option<Person> {
        self.device_person(req.extensions().get().copied())
            .or_else(|| {
                session::cookies(req.headers(), APP_COOKIE)
                    .find_map(|sid| self.sessions.app_user(sid, host))
            })
    }

    /// 从 `owner.json` / `members.json` 取本人资料；插件出口只用用户名派生内部邮箱，不公开真实邮箱。
    /// 复用 [`Owner`] 值类型；家人的记录找不到就报错，不编一个身份出来。
    pub(crate) fn identity_of(&self, person: &Person) -> Result<Owner, MissingMember> {
        if person.is_owner() {
            return Ok(self.owner.owner());
        }
        if person.role == Role::Friend {
            let record = self
                .sharing
                .as_ref()
                .and_then(|sharing| sharing.friends.find_username(&person.username))
                .ok_or_else(|| MissingMember(person.username.clone()))?;
            return Ok(Owner {
                username: record.username,
                display_name: record.display_name.unwrap_or_default(),
                email: None,
            });
        }
        let record = self
            .members
            .get(&person.username)
            .ok_or_else(|| MissingMember(person.username.clone()))?;
        Ok(Owner {
            username: record.username,
            display_name: record.display_name,
            email: record.email,
        })
    }

    /// 这台设备的人在 `GET ME_PATH` 里看到的自己（ADR-0045 的 `Me`）。家人的记录找不到了就报错：
    /// 设备刚被移除时会这样，不编一个身份出来。
    pub(crate) fn me(&self, person: &Person) -> Result<wonton_protocol::Me, MissingMember> {
        let user_id = self
            .matrix
            .as_ref()
            .map(|m| m.user_id(&person.username))
            .unwrap_or_else(|| format!("@{}:{}", person.username, self.public.base_domain));
        if person.is_owner() {
            let profile = self.owner.profile();
            Ok(wonton_protocol::Me {
                user_id,
                username: profile.username,
                is_owner: true,
                role_id: None,
                role_name: None,
                permissions: None,
                display_name: profile.display_name,
                email: profile.email,
            })
        } else {
            let record = self
                .members
                .get(&person.username)
                .ok_or_else(|| MissingMember(person.username.clone()))?;
            if record.status == MemberStatus::Removed {
                return Err(MissingMember(person.username.clone()));
            }
            // 角色记录读不到时按最严的权限报，不冒充能给：`role_id` 照实给出记录里那条。
            let role = self.roles.get(&record.role_id);
            Ok(wonton_protocol::Me {
                user_id,
                username: record.username,
                is_owner: false,
                role_name: role
                    .as_ref()
                    .map(|role| role.name.trim())
                    .filter(|name| !name.is_empty())
                    .map(str::to_owned),
                permissions: Some(
                    role.as_ref()
                        .map_or_else(Permissions::strictest, |role| role.permissions.clone()),
                ),
                role_id: Some(record.role_id),
                display_name: Some(record.display_name),
                email: record.email,
            })
        }
    }

    /// 这个人现在的权限：每次请求按记录里的角色解析，角色改了下一个请求就生效。主人短路为全部权限；
    /// 记录被移除、或者记录里的角色不在了，按最严的 [`Permissions::strictest`] 算，不冒充能给。
    pub(crate) fn permissions(&self, person: &Person) -> Permissions {
        if person.is_owner() {
            return Permissions::owner();
        }
        let Some(record) = self.members.get(&person.username) else {
            return Permissions::strictest();
        };
        if record.status == MemberStatus::Removed {
            return Permissions::strictest();
        }
        self.roles
            .get(&record.role_id)
            .map_or_else(Permissions::strictest, |role| role.permissions)
    }

    /// 这个角色有几人在用：没移除的家人和还在名单里的好友。角色接口接上之前只有测试用。
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn role_people(&self, role_id: &str) -> u32 {
        let members = self
            .members
            .list()
            .iter()
            .filter(|m| m.status != MemberStatus::Removed && m.role_id == role_id)
            .count();
        let friends = self.friends.as_ref().map_or(0, |friends| {
            friends
                .list_active()
                .iter()
                .filter(|f| f.role_id == role_id)
                .count()
        });
        u32::try_from(members + friends).unwrap_or(u32::MAX)
    }

    /// 删一个角色。用的人数按这一刻的记录算：先把人换走，再删。角色接口接上之前只有测试用。
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn delete_role(&self, role_id: &str) -> Result<(), RolesError> {
        self.roles.delete(role_id, self.role_people(role_id))
    }

    fn secure_cookies(&self) -> bool {
        self.public.scheme == "https"
    }

    fn core_origin(&self) -> String {
        self.public.origin(self.public.core_host())
    }
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
        HostKind::Matrix => matrix::client_request(&gateway, req).await,
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
mod friends_tests;
#[cfg(test)]
mod health_tests;
#[cfg(test)]
mod isolation_policy_tests;
#[cfg(test)]
mod isolation_tests;
#[cfg(test)]
mod join_recovery_tests;
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
mod matrix_invite_filter_tests;
#[cfg(test)]
mod matrix_rtc_docker_tests;
#[cfg(test)]
mod matrix_rtc_tests;
#[cfg(test)]
mod mcp_takeover_tests;
#[cfg(test)]
mod mcp_tests;
#[cfg(test)]
mod model_providers_tests;
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
mod panel_pins_tests;
#[cfg(test)]
mod peers_api_tests;
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
