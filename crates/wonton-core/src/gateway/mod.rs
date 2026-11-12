//! 核心网关：按 Host 头把请求分给核心自己、身份提供方或插件网页，所有插件请求先鉴权。
//!
//! | Host | 去向 |
//! | --- | --- |
//! | `<base>` | 核心：登录、状态页、插件 API、OIDC 登录与授权确认端点 |
//! | `auth.<base>` | 身份提供方（Ory Hydra）的公开端点 |
//! | `matrix.<base>` | Matrix 客户端接口，只给经隧道的已配对设备（`matrix`） |
//! | `push.wonton.internal` | 推送代理，只给 Tuwunel（`push`） |
//! | `notify.wonton.internal` | 插件发通知进"Wonton"私聊，只放行申请了 `notify` 的插件（`notify`） |
//! | `<slug>.<base>` | 插件网页，需要插件会话 |
//!
//! 经隧道接入的已配对设备不需要 Cookie：隧道入口（`crate::tunnel`）把设备身份放进请求扩展
//! （[`TunnelPeer`]），网关据此直接认出主人。局域网浏览器仍走核心会话加插件会话交接。
//! 本地代理可能被同机其他 App 借用，所以设备一侧的本地代理要校验会话凭据（ADR-0004、#20）；
//! 网关只在插件网页、OIDC 登录和授权确认端点、Matrix 登录令牌、Matrix 客户端接口、文件接口和插件管理接口（不能是网页
//! 发起的，ADR-0017）上接受隧道身份，其余管理 API 仍要主人令牌。
//!
//! 插件子域名彼此同站，也和核心同站：插件网页能让浏览器带着 Cookie 或隧道身份向别处发请求。浏览器里的
//! 隔离规则见 `isolation`：插件上只认同源请求和导航，核心上改状态的请求只接受同源，插件的 Cookie 出不了
//! 自己的子域名。

mod agent_permission;
mod api;
mod api_body;
mod bot_profile;
mod confirmations;
mod core_routes;
mod core_upgrade;
mod devices;
mod files;
mod health;
mod isolation;
mod lan;
pub mod matrix;
mod mcp;
mod model_providers;
mod models;
mod notify;
mod notify_limit;
mod onboarding;
mod owner_profile;
mod pages;
mod peers;
mod plugin_start;
mod proxy;
mod proxy_sso;
mod push;
mod pushers;
mod registry_mirror;
mod room_rules;
pub(crate) mod send_limits;

pub use send_limits::{SendLimiter, SendLimits};
mod session;
mod sso;
mod tool_policy;

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use axum::extract::{ConnectInfo, Request};
use axum::response::{IntoResponse, Redirect, Response};
use hyper::header::{ACCEPT, HOST, SET_COOKIE};
use hyper::{HeaderMap, Method, StatusCode};
use iroh::EndpointId;
use tower::ServiceExt;

use crate::config::{self, HostKind, PublicUrls};
use crate::http::HttpClient;
use crate::l10n;
use crate::matrix::Matrix;
use crate::oidc::Hydra;
use crate::owner::OwnerStore;
use crate::plugin::{PluginManager, PluginStatus};
use crate::push::Push;
use crate::secret::constant_time_eq;
use crate::tunnel::Tunnel;

pub use pages::ERROR_HEADER;
pub use session::{APP_COOKIE, CORE_COOKIE};

/// 经隧道接入的设备：QUIC 握手认证过的 endpoint ID。只有核心的隧道入口会把它放进请求扩展，
/// 局域网 TCP 请求里没有，客户端也没法用请求头伪造。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TunnelPeer(pub EndpointId);

pub struct Gateway {
    pub public: PublicUrls,
    pub owner: Arc<OwnerStore>,
    owner_token: String,
    pub plugins: Arc<PluginManager>,
    pub hydra: Option<Hydra>,
    /// 身份提供方公开端点的内部地址（`host:port`），`auth.<base>` 的请求转发到这里。
    pub hydra_public: Option<String>,
    pub http: HttpClient,
    system_http: HttpClient,
    pub tunnel: Tunnel,
    /// Matrix 服务器；没接时 App 的 Matrix 入口如实报不可用。
    pub matrix: Option<Arc<Matrix>>,
    /// Bot 发消息的频率和跳数（ADR-0045）。计数在内存里，核心重启从头算。
    pub(crate) send_limits: Arc<send_limits::SendLimiter>,
    /// 推送代理；没启用时登记和推送都如实报不可用。
    pub push: Option<Arc<Push>>,
    /// MCP 总线；没启用时 `mcp.wonton.internal` 如实报不可用。
    pub mcp: Option<crate::mcp::Endpoint>,
    /// 通知房间；没接 Matrix 时插件发通知如实报不可用。
    pub notices: Option<Arc<crate::matrix::NoticeRoom>>,
    /// 插件通知的频率限制（ADR-0047），计数只在内存里。
    notify_limiter: notify_limit::Limiter,
    /// 文件服务；没启用时文件接口如实报不可用。
    pub files: Option<Arc<crate::files::Files>>,
    /// tusd 的 `host:port`；没配置时上传如实报不可用。
    pub tusd: Option<String>,
    /// 核心的一键升级（ADR-0025）；没接时接口如实报不可用。
    pub upgrades: Option<Arc<crate::upgrade::service::Upgrades>>,
    /// 服务器配对（ADR-0050）；没接 Matrix 的核心没有它，接口如实报不可用。
    pub peers: Option<Arc<crate::peers::Peers>>,
    pub(crate) onboarding: Arc<onboarding::OnboardingState>,
    /// `plugins.health` 的限频表（[`health`]）。
    health_limits: health::RateLimits,
    sessions: session::SessionStore,
}

impl Gateway {
    /// `hydra` 带上身份提供方公开端点的地址，必须来自 [`crate::config::Config`]：配置层已经校验过它是
    /// `http://主机:端口`。
    pub fn new(
        public: PublicUrls,
        owner: Arc<OwnerStore>,
        owner_token: String,
        plugins: Arc<PluginManager>,
        hydra: Option<(Hydra, String)>,
        http: HttpClient,
        tunnel: Tunnel,
    ) -> Self {
        let (hydra, hydra_public) = match hydra {
            Some((hydra, public_url)) => {
                let authority = config::http_authority(&public_url)
                    .unwrap_or_else(|e| panic!("身份提供方的公开地址没有经过配置层校验：{e}"));
                (Some(hydra), Some(authority))
            }
            None => (None, None),
        };
        let onboarding = Arc::new(onboarding::OnboardingState::new(plugins.data_dir()));
        Self {
            public,
            owner,
            owner_token,
            plugins,
            hydra,
            hydra_public,
            http,
            system_http: HttpClient::system_loopback(),
            tunnel,
            matrix: None,
            send_limits: Arc::new(send_limits::SendLimiter::new(
                send_limits::SendLimits::default(),
            )),
            push: None,
            mcp: None,
            notices: None,
            notify_limiter: notify_limit::Limiter::default(),
            files: None,
            tusd: None,
            upgrades: None,
            peers: None,
            onboarding,
            health_limits: health::RateLimits::new(),
            sessions: session::SessionStore::default(),
        }
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

    /// 经隧道接入的已配对设备属于主人（M0 只有主人一个用户）。吊销后立即不再认它，哪怕请求还在
    /// 一条没来得及断开的连接上。
    fn device_user(&self, peer: Option<TunnelPeer>) -> Option<String> {
        let TunnelPeer(id) = peer?;
        self.tunnel
            .devices
            .contains(&id)
            .then(|| self.owner.username().to_owned())
    }

    /// 核心域名上的当前用户（M0 只有主人）：经隧道的已配对设备，或者局域网浏览器的核心会话。
    fn core_user(&self, peer: Option<TunnelPeer>, headers: &HeaderMap) -> Option<String> {
        self.device_user(peer).or_else(|| {
            session::cookies(headers, CORE_COOKIE).find_map(|sid| self.sessions.core_user(sid))
        })
    }

    /// 插件子域名上的当前用户：经隧道的已配对设备，或者这个子域名的插件会话。只在
    /// [`isolation::credentials_allowed`] 放行的请求上调用。
    fn plugin_user(&self, host: &str, req: &Request) -> Option<String> {
        self.device_user(req.extensions().get().copied())
            .or_else(|| {
                session::cookies(req.headers(), APP_COOKIE)
                    .find_map(|sid| self.sessions.app_user(sid, host))
            })
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
    let host = request_host(&req);
    match gateway.public.classify(&host) {
        HostKind::Core => match core.oneshot(req).await {
            Ok(response) => response,
            Err(never) => match never {},
        },
        HostKind::Auth => auth_request(&gateway, req).await,
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

/// `auth.<base>`：身份提供方的公开端点（授权、令牌、用户信息、JWKS、发现文档）。
/// 这些端点本身就面向公网设计，由协议保护；真正的用户鉴权发生在核心的登录端点。
async fn auth_request(gateway: &Gateway, req: Request) -> Response {
    let langs = api::languages(req.headers());
    let Some(authority) = &gateway.hydra_public else {
        return pages::error(
            StatusCode::SERVICE_UNAVAILABLE,
            "oidc_unavailable",
            &l10n::text(&langs, "gateway-oidc-unconfigured", &[]),
            &langs,
        );
    };
    let fwd = proxy::Forwarded {
        client: client_addr(&req),
        scheme: &gateway.public.scheme,
    };
    let endpoint = crate::plugin::upstreams::Endpoint::Tcp(authority.clone());
    let name = l10n::text(&langs, "gateway-name-oidc", &[]);
    let target = proxy::Target {
        endpoint: &endpoint,
        name: &name,
    };
    proxy::forward(&gateway.system_http, target, req, fwd, &langs).await
}

/// 插件子域名：经隧道的已配对设备直接放行；局域网浏览器先兑换交接码或校验插件会话。登录状态只认本插件
/// 页面发起的请求和导航。`sso: proxy` 的插件这时加上代登录凭据（`proxy_sso`）。然后转发给插件，并过滤插件
/// 响应里的 `Set-Cookie` 和会交出凭据的头。
async fn plugin_request(gateway: &Gateway, slug: &str, mut req: Request) -> Response {
    let host = gateway.public.plugin_host(slug);
    let langs = api::languages(req.headers());
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
        );
    };
    if req.uri().path() == "/_wonton/session" {
        return sso::redeem(gateway, &host, &req);
    }
    let origin = gateway.public.origin(&host);
    let credentials = isolation::credentials_allowed(req.method(), req.headers(), &origin);
    if !credentials || gateway.plugin_user(&host, &req).is_none() {
        return unauthenticated(gateway, &host, &req, credentials);
    }
    let upstream = match (&route.status, &route.upstream) {
        (PluginStatus::Running, Some(upstream)) => upstream.clone(),
        (PluginStatus::Running, None) => {
            return pages::error(
                StatusCode::NOT_FOUND,
                "no_webui",
                &l10n::text(&langs, "gateway-no-webui", &[]),
                &langs,
            );
        }
        // 辅助容器坏了：网页服务自己的容器在运行、健康检查没失败就照常转发，网页里依赖坏掉的那部分由
        // 应用自己报错。插件整体照旧是异常，App 里能看到是哪个容器。
        (PluginStatus::Degraded { .. }, Some(upstream)) if route.web_serving => upstream.clone(),
        (status, _) => {
            let detail = pages::state_of(status, &langs);
            return pages::error_with_detail(
                StatusCode::SERVICE_UNAVAILABLE,
                "not_ready",
                &l10n::text(
                    &langs,
                    "gateway-app-not-ready",
                    &[("id", route.id.as_str().into())],
                ),
                Some(&detail),
                &langs,
            );
        }
    };
    if let Some(auth) = &route.proxy
        && let Err(e) = proxy_sso::attach(req.headers_mut(), auth, &gateway.owner.owner(), &langs)
    {
        return pages::error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "proxy_credentials",
            &l10n::text(
                &langs,
                "gateway-proxy-sso-failed",
                &[
                    ("id", route.id.as_str().into()),
                    ("error", e.to_string().into()),
                ],
            ),
            &langs,
        );
    }
    let fwd = proxy::Forwarded {
        client: client_addr(&req),
        scheme: &gateway.public.scheme,
    };
    let target = proxy::Target {
        endpoint: &upstream,
        name: route.id.as_str(),
    };
    let mut response = match proxy::forward_or(&gateway.http, target, req, fwd, &langs).await {
        Ok(response) => response,
        Err(error) => return error.response(route.id.as_str(), &langs),
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

/// 未登录：浏览器的页面请求跳到核心换取插件会话，其余请求（XHR、API）直接 401。`credentials` 为假的
/// 跨站请求换到会话也不认，同样 401。
fn unauthenticated(gateway: &Gateway, host: &str, req: &Request, credentials: bool) -> Response {
    let langs = api::languages(req.headers());
    if !credentials {
        return pages::error(
            StatusCode::UNAUTHORIZED,
            "unauthenticated",
            &l10n::text(&langs, "gateway-cross-site-login", &[]),
            &langs,
        );
    }
    let wants_html = req
        .headers()
        .get(ACCEPT)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.contains("text/html"));
    if req.method() == Method::GET && wants_html {
        let path = req.uri().path_and_query().map_or("/", |p| p.as_str());
        let to = format!("{}{path}", gateway.public.origin(host));
        let query = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("to", &to)
            .finish();
        return Redirect::to(&format!("{}/_wonton/sso?{query}", gateway.core_origin()))
            .into_response();
    }
    pages::error(
        StatusCode::UNAUTHORIZED,
        "unauthenticated",
        &l10n::text(&langs, "gateway-login-required", &[]),
        &langs,
    )
}

/// 把跳转目标相对 `origin` 解析，只接受同源结果，返回站内路径（含查询）。浏览器会把 http(s)
/// 地址里的 `\` 当成 `/`，只拒绝 `//` 开头不够（`/\evil.example` 会跳到 `http://evil.example`）。
fn same_origin_path(origin: &str, target: &str) -> Option<String> {
    let base = url::Url::parse(origin).ok()?;
    let joined = base.join(target).ok()?;
    (joined.origin() == base.origin()).then(|| joined[url::Position::BeforePath..].to_owned())
}

fn with_cookie(mut response: Response, cookie: String) -> Response {
    if let Ok(value) = cookie.parse() {
        response.headers_mut().append(SET_COOKIE, value);
    }
    response
}

#[cfg(test)]
mod bot_profile_tests;
#[cfg(test)]
mod core_upgrade_tests;
#[cfg(test)]
mod degraded_webui_tests;
#[cfg(test)]
mod egress_api_tests;
#[cfg(test)]
mod family_rules_tests;
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
mod owner_profile_tests;
#[cfg(test)]
mod pairing_tests;
#[cfg(test)]
mod peers_api_tests;
#[cfg(test)]
mod plugin_start_tests;
#[cfg(test)]
mod proxy_sso_tests;
#[cfg(test)]
mod push_tests;
#[cfg(test)]
mod registry_mirror_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tunnel_tests;
#[cfg(test)]
mod tusd_tests;
