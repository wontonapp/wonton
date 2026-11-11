//! 核心网关：按 Host 头把请求分给核心自己、身份提供方或插件网页，所有插件请求先鉴权。
//!
//! | Host | 去向 |
//! | --- | --- |
//! | `<base>` | 核心：登录、状态页、插件 API、OIDC 登录与授权确认端点 |
//! | `auth.<base>`、`auth.wonton.internal` | 身份提供方（Ory Hydra）的公开端点，发现文档按谁访问改写（ADR-0063） |
//! | `matrix.<base>` | Matrix 客户端接口，只给经隧道的已配对设备（`matrix`） |
//! | `push.wonton.internal` | 推送代理，只给 Tuwunel（`push`） |
//! | `notify.wonton.internal` | 插件发通知进"Wonton"私聊，只放行申请了 `notify` 的插件（`notify`） |
//! | `<slug>.<base>` | 插件网页，需要插件会话；这个插件自己网络里的容器访问它时不要（ADR-0063） |
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

mod agent_permission;
mod api;
mod api_body;
mod api_switch;
mod auth;
mod bot_activity;
mod bot_contacts;
mod bot_profile;
mod bots;
mod confirmations;
mod core_routes;
mod core_upgrade;
mod devices;
mod entry;
pub mod family;
mod files;
mod friends;
mod friends_wiring;
mod health;
mod isolation;
mod lan;
mod link_preview;
pub mod matrix;
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
mod plugin_scope;
mod plugin_start;
mod plugin_web;
mod proxy;
mod proxy_sso;
mod push;
mod pushers;
mod registry_mirror;
mod request_guards;
mod room_rules;
pub(crate) mod send_limits;
pub mod sharing;

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
use axum::response::{IntoResponse, Redirect, Response};
use hyper::header::{ACCEPT_LANGUAGE, AUTHORIZATION, HOST, SET_COOKIE};
use hyper::{HeaderMap, Method, StatusCode};
use iroh::EndpointId;
use tower::ServiceExt;
use wonton_protocol::{MemberStatus, Role};

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
    friends: Option<Arc<Friends>>,
    /// Bot 的联系人（ADR-0045、ADR-0050）；没接 Matrix 的核心没有。
    contacts: Option<Arc<crate::contacts::Contacts>>,
    pub(crate) federation_blocks: Option<Arc<FederationBlocks>>,
    pub sharing: Option<Arc<sharing::Sharing>>,
    pub(crate) onboarding: Arc<onboarding::OnboardingState>,
    health_limits: health::RateLimits,
    pub(crate) link_previews: Arc<crate::link_preview::LinkPreviews>,
    pub(crate) app_calls: Arc<mcp_apps::AppCalls>,
    sessions: session::SessionStore,
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
        Self {
            public,
            owner,
            owner_token,
            members,
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
            sessions: session::SessionStore::default(),
        }
    }

    #[must_use]
    pub fn with_sharing(mut self, sharing: Option<Arc<sharing::Sharing>>) -> Self {
        self.sharing = sharing;
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
            Some(record) if record.status != MemberStatus::Removed => {
                Some(Person::member(&record.username))
            }
            _ => {
                tracing::warn!(
                    device = %id.fmt_short(),
                    member = %username,
                    "设备的家人不在记录里或已移除，拒绝这台设备的请求"
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

    /// 插件子域名上的当前人：经隧道的已配对设备，或者这个子域名的插件会话。只在
    /// [`isolation::credentials_allowed`] 放行的请求上调用。
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
                role: Role::Owner,
                display_name: profile.display_name,
                email: profile.email,
                can_install_plugins: true,
            })
        } else {
            let record = self
                .members
                .get(&person.username)
                .ok_or_else(|| MissingMember(person.username.clone()))?;
            if record.status == MemberStatus::Removed {
                return Err(MissingMember(person.username.clone()));
            }
            Ok(wonton_protocol::Me {
                user_id,
                username: record.username,
                role: Role::Member,
                display_name: Some(record.display_name),
                email: record.email,
                can_install_plugins: record.can_install_plugins,
            })
        }
    }

    /// 这个人能不能装卸、升级插件：主人总是能，家人要主人授权（ADR-0045）。
    pub(crate) fn can_install_plugins(&self, person: &Person) -> bool {
        person.is_owner()
            || self
                .members
                .get(&person.username)
                .is_some_and(|record| record.can_install_plugins)
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
        HostKind::Push => push::gateway_request(&gateway, req).await,
        HostKind::PluginMcp => mcp::request(&gateway, req).await,
        HostKind::PluginHealth => health::request(&gateway, req).await,
        HostKind::PluginNotify => notify::request(&gateway, req).await,
        HostKind::FileHooks => files::hook_request(&gateway, req).await,
        HostKind::Plugin(slug) => plugin_request(&gateway, &slug, req).await,
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

/// 插件子域名：经隧道的已配对设备直接放行；局域网浏览器先兑换交接码或校验插件会话。登录状态只认本插件
/// 页面发起的请求和导航。`sso: proxy` 的插件这时加上代登录凭据（`proxy_sso`）。然后转发给插件，并过滤插件
/// 响应里的 `Set-Cookie` 和会交出凭据的头。
async fn plugin_request(gateway: &Gateway, slug: &str, mut req: Request) -> Response {
    let host = gateway.public.plugin_host(slug);
    let langs = api::languages(req.headers());
    let format = pages::Format::of(req.headers());
    let Some(route) = gateway.plugins.route(slug) else {
        return pages::error(
            StatusCode::NOT_FOUND,
            "not_installed",
            &l10n::text(
                &langs,
                "gateway-no-app-for-host",
                &[("host", host.as_str().into())],
            ),
            &langs,
            format,
        );
    };
    // 这个插件自己网络里的容器访问它自己的网页地址：不要会话，转给它自己的上游，也不代表任何人（ADR-0063）。
    // 认来源用网关认出调用方的同一张表（按来源地址落在哪个插件网络里）。
    let from_itself = client_addr(&req)
        .and_then(|addr| gateway.plugins.source(addr.ip()))
        .is_some_and(|source| source.id == route.id);
    if from_itself {
        return plugin_web::self_request(gateway, &route, req).await;
    }
    if req.uri().path() == "/_wonton/session" {
        return sso::redeem(gateway, &host, &req);
    }
    let origin = gateway.public.origin(&host);
    let credentials = isolation::credentials_allowed(req.method(), req.headers(), &origin);
    let person = gateway.plugin_person(&host, &req);
    if !credentials {
        return request_guards::unauthenticated(gateway, &host, &req, false);
    }
    let Some(person) = person else {
        return request_guards::unauthenticated(gateway, &host, &req, true);
    };
    // 插件对家人只看“家人能用”开关：主人开放了才放行（ADR-0045）。核心不查 setup_done（ADR-0021），
    // 家人这一侧没有别的门槛。
    if !person.is_owner() && !gateway.plugins.family_enabled(&route.id) {
        return pages::error(
            StatusCode::FORBIDDEN,
            "not_shared",
            &l10n::text(
                &langs,
                "gateway-plugin-not-shared",
                &[("id", route.id.as_str().into())],
            ),
            &langs,
            format,
        );
    }
    let upstream = match plugin_web::upstream(&route, &langs, format) {
        Ok(upstream) => upstream,
        Err(response) => return *response,
    };
    if let Err(response) =
        attach_proxy_credentials(gateway, &route, &person, req.headers_mut(), &langs, format)
    {
        return *response;
    }
    let fwd = proxy::Forwarded {
        client: client_addr(&req),
        scheme: &gateway.public.scheme,
    };
    let target = proxy::Target {
        endpoint: &upstream,
        name: route.id.as_str(),
    };
    let mut response =
        match proxy::forward_or(&gateway.upstream_http, target, req, fwd, &langs).await {
            Ok(response) => response,
            Err(error) => return error.response(route.id.as_str(), &langs, format),
        };
    isolation::sanitize_set_cookies(response.headers_mut());
    if let Some(auth) = &route.proxy {
        proxy_sso::scrub(response.headers_mut(), auth);
    }
    if route.cross_origin_isolated {
        proxy::add_document_isolation_policy(&mut response);
    }
    response
}

/// 代登录（`sso: proxy`）：拿这个人的身份写进请求头。拿不到身份或值写不进去时，回错误（形状按请求协商）。
fn attach_proxy_credentials(
    gateway: &Gateway,
    route: &crate::plugin::manager::Route,
    person: &Person,
    headers: &mut hyper::HeaderMap,
    langs: &crate::plugin::languages::Languages,
    format: pages::Format,
) -> Result<(), Box<Response>> {
    let Some(auth) = &route.proxy else {
        return Ok(());
    };
    let identity = match gateway.identity_of(person) {
        Ok(identity) => identity,
        Err(e) => {
            tracing::warn!(plugin = %route.id, error = %e, "代登录拿不到身份");
            return Err(Box::new(pages::error(
                StatusCode::FORBIDDEN,
                "permission_denied",
                &l10n::text(langs, "gateway-proxy-sso-member-missing", &[]),
                langs,
                format,
            )));
        }
    };
    proxy_sso::attach(headers, auth, &identity, person.role).map_err(|e| {
        Box::new(pages::error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "proxy_credentials",
            &l10n::text(
                langs,
                "gateway-proxy-sso-failed",
                &[
                    ("id", route.id.as_str().into()),
                    ("error", e.to_string().into()),
                ],
            ),
            langs,
            format,
        ))
    })
}

/// 未登录：浏览器的页面导航跳到核心换取插件会话，其余请求（XHR、API）直接 401。`credentials` 为假的
/// 跨站请求换到会话也不认，同样 401。
#[cfg(test)]
mod bot_activity_tests;
#[cfg(test)]
mod bot_contacts_tests;
#[cfg(test)]
mod bot_profile_tests;
#[cfg(test)]
mod bots_tests;
#[cfg(test)]
mod core_upgrade_tests;
#[cfg(test)]
mod degraded_webui_tests;
#[cfg(test)]
mod egress_api_tests;
#[cfg(test)]
pub(crate) mod fakes;
#[cfg(test)]
mod family_plugin_tests;
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
mod l10n_tests;
#[cfg(test)]
mod lan_api_tests;
#[cfg(test)]
mod lan_tests;
#[cfg(test)]
mod limits_tests;
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
