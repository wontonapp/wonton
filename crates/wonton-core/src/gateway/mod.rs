//! 核心网关：按 Host 头把请求分给核心自己、身份提供方或插件网页，所有插件请求先鉴权。
//!
//! | Host | 去向 |
//! | --- | --- |
//! | `<base>` | 核心：登录、状态页、插件 API、OIDC 登录与授权确认端点 |
//! | `auth.<base>` | 身份提供方（Ory Hydra）的公开端点 |
//! | `<slug>.<base>` | 插件网页，需要插件会话 |
//!
//! 设备经隧道接入后（#2），隧道层会在请求扩展里放入已认证的身份，届时插件请求不再需要 Cookie。

mod api;
mod core_routes;
mod pages;
mod proxy;
mod session;
mod sso;

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use axum::extract::{ConnectInfo, Request};
use axum::response::{IntoResponse, Redirect, Response};
use hyper::header::{ACCEPT, HOST, SET_COOKIE};
use hyper::{HeaderMap, Method, StatusCode};
use tower::ServiceExt;

use crate::config::{HostKind, Owner, PublicUrls};
use crate::http::HttpClient;
use crate::oidc::Hydra;
use crate::plugin::{PluginManager, PluginStatus};
use crate::secret::constant_time_eq;

pub use pages::ERROR_HEADER;
pub use session::{APP_COOKIE, CORE_COOKIE};

pub struct Gateway {
    pub public: PublicUrls,
    pub owner: Owner,
    owner_token: String,
    pub plugins: Arc<PluginManager>,
    pub hydra: Option<Hydra>,
    /// 身份提供方公开端点的内部地址（`host:port`），`auth.<base>` 的请求转发到这里。
    pub hydra_public: Option<String>,
    pub http: HttpClient,
    sessions: session::SessionStore,
}

impl Gateway {
    pub fn new(
        public: PublicUrls,
        owner: Owner,
        owner_token: String,
        plugins: Arc<PluginManager>,
        hydra: Option<(Hydra, String)>,
        http: HttpClient,
    ) -> Self {
        let (hydra, hydra_public) = match hydra {
            Some((hydra, public_url)) => {
                let authority = public_url
                    .trim_start_matches("http://")
                    .trim_end_matches('/')
                    .to_owned();
                (Some(hydra), Some(authority))
            }
            None => (None, None),
        };
        Self {
            public,
            owner,
            owner_token,
            plugins,
            hydra,
            hydra_public,
            http,
            sessions: session::SessionStore::default(),
        }
    }

    fn check_owner_token(&self, token: &str) -> bool {
        constant_time_eq(token.trim(), &self.owner_token)
    }

    /// 核心域名上的当前用户（M0 只有主人）。
    fn core_user(&self, headers: &HeaderMap) -> Option<String> {
        let sid = session::cookie(headers, CORE_COOKIE)?;
        self.sessions.core_user(&sid)
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
        HostKind::Plugin(slug) => plugin_request(&gateway, &slug, req).await,
        HostKind::Unknown => pages::error(
            StatusCode::NOT_FOUND,
            "unknown_host",
            &format!("未知的地址 {host}"),
        ),
    }
}

fn client_addr(req: &Request) -> Option<SocketAddr> {
    req.extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ConnectInfo(addr)| *addr)
}

/// `auth.<base>`：身份提供方的公开端点（授权、令牌、用户信息、JWKS、发现文档）。
/// 这些端点本身就面向公网设计，由协议保护；真正的用户鉴权发生在核心的登录端点。
async fn auth_request(gateway: &Gateway, req: Request) -> Response {
    let Some(authority) = &gateway.hydra_public else {
        return pages::error(
            StatusCode::SERVICE_UNAVAILABLE,
            "oidc_unavailable",
            "核心没有配置身份提供方",
        );
    };
    let fwd = proxy::Forwarded {
        client: client_addr(&req),
        scheme: &gateway.public.scheme,
    };
    let target = proxy::Target {
        authority,
        name: "身份提供方",
    };
    proxy::forward(&gateway.http, target, req, fwd).await
}

/// 插件子域名：先兑换交接码或校验插件会话，再转发给插件。
async fn plugin_request(gateway: &Gateway, slug: &str, req: Request) -> Response {
    let host = gateway.public.plugin_host(slug);
    let Some(route) = gateway.plugins.route(slug) else {
        return pages::error(
            StatusCode::NOT_FOUND,
            "not_installed",
            &format!("没有安装对应 {host} 的插件"),
        );
    };
    if req.uri().path() == "/_wonton/session" {
        return sso::redeem(gateway, &host, &req);
    }
    let user = session::cookie(req.headers(), APP_COOKIE)
        .and_then(|sid| gateway.sessions.app_user(&sid, &host));
    if user.is_none() {
        return unauthenticated(gateway, &host, &req);
    }
    let upstream = match (&route.status, &route.upstream) {
        (PluginStatus::Running, Some(upstream)) => upstream.clone(),
        (PluginStatus::Running, None) => {
            return pages::error(StatusCode::NOT_FOUND, "no_webui", "这个插件没有网页入口");
        }
        (status, _) => {
            return pages::error(
                StatusCode::SERVICE_UNAVAILABLE,
                "not_ready",
                &format!("插件 {} 尚未就绪：{status:?}", route.id),
            );
        }
    };
    let fwd = proxy::Forwarded {
        client: client_addr(&req),
        scheme: &gateway.public.scheme,
    };
    let target = proxy::Target {
        authority: &upstream,
        name: route.id.as_str(),
    };
    proxy::forward(&gateway.http, target, req, fwd).await
}

/// 未登录：浏览器的页面请求跳到核心换取插件会话，其余请求（XHR、API）直接 401。
fn unauthenticated(gateway: &Gateway, host: &str, req: &Request) -> Response {
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
        "需要先登录 Wonton",
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
mod tests;
