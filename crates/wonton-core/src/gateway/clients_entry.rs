//! 8484 上插件主机名的转发（ADR-0021「核心给入口容器开一个专用监听端口」、ADR-0090）：开关、入口头、
//! 公网路径规则、`setup_done`、头和 Cookie 的清理都在这里。对端是不是入口容器由 `entry_runtime` 的
//! `trusted_entry` 中间件把关；`/_wonton/` 前缀在插件主机名上归核心（ADR-0080），一律 404。

use std::net::{IpAddr, SocketAddr};

use axum::extract::Request;
use axum::response::Response;
use hyper::StatusCode;
use hyper::header::{self, HeaderMap, HeaderValue};
use tokio::sync::watch;

use super::matrix::matrix_error;
use super::{Gateway, api, pages, proxy, session};
use crate::clients::gateway_auth::{self, Attempt};
use crate::clients::path_rules::{self, PathRules};
use crate::clients::state::Kind;
use crate::entry_haproxy::model::AUTH_FAILURE_HEADER;
use crate::l10n;
use crate::plugin::languages::Languages;
use crate::plugin::manager::Route;
use crate::plugin::proxy_auth::ProxyAuth;

/// 8484 上的错误都走网关的形状（类别在响应头里，文案四种语言）；只有「这个名字这里不伺候」的 421
/// 沿用监听里 Matrix 的形状：名字空间是入口容器按 crt-list 分的，到不了这里的多半不是插件的名字。
fn misdirected() -> Response {
    matrix_error(
        StatusCode::MISDIRECTED_REQUEST,
        "M_UNRECOGNIZED",
        "invalid_host",
        "入口只伺候本机服务器名和插件的入口名字",
    )
}

/// 插件主机名：`<标签>.<后缀>` 是公网入口，`<标签>.lan.<后缀>` 是局域网入口，带端口的形式照认
/// （入口容器已把 Host 的主机段和 SNI 比过）。其余——外来域、裸后缀、多段标签——都不是插件名字。
pub(super) fn split_host(req: &Request, suffix: &str) -> Option<(String, Kind)> {
    let mut hosts = req.headers().get_all(header::HOST).iter();
    let host = hosts.next()?.to_str().ok()?;
    if hosts.next().is_some() {
        return None;
    }
    let name = match host.rsplit_once(':') {
        Some((name, port))
            if !port.is_empty()
                && port.bytes().all(|b| b.is_ascii_digit())
                && !name.contains(':') =>
        {
            name
        }
        _ => host,
    };
    if name.is_empty() || name.contains('@') || name.ends_with('.') {
        return None;
    }
    let name = name.to_ascii_lowercase();
    let suffix = suffix.to_ascii_lowercase();
    if let Some(label) = name.strip_suffix(&format!(".lan.{suffix}")) {
        return single_label(label).map(|label| (label, Kind::Lan));
    }
    let label = name.strip_suffix(&format!(".{suffix}"))?;
    single_label(label).map(|label| (label, Kind::Public))
}

fn single_label(label: &str) -> Option<String> {
    (!label.is_empty() && !label.contains('.')).then(|| label.to_owned())
}

/// 入口容器的标记恰好一个，取值和主机名的形式对得上：`.lan.` ↔ `lan`（entry_haproxy 的前端名）。
fn entry_header_matches(req: &Request, kind: Kind) -> bool {
    let mut entries = req.headers().get_all("x-wonton-entry").iter();
    let Some(value) = entries.next().and_then(|v| v.to_str().ok()) else {
        return false;
    };
    if entries.next().is_some() {
        return false;
    }
    value == kind.header_value()
}

pub(super) async fn dispatch(
    gateway: &Gateway,
    req: Request,
    label: String,
    kind: Kind,
) -> Response {
    let langs = api::languages(req.headers());
    let format = pages::Format::of(req.headers());
    // 访客链接（C18）还没来；这个前缀是核心的，不问开关、不转发，也不借它探测标签和开关。
    if path_rules::is_reserved_path(req.uri().path()) {
        return deny(
            StatusCode::NOT_FOUND,
            "not_found",
            "gateway-not-found",
            &langs,
            format,
        );
    }
    let resolved = match gateway.clients.resolve(&label, kind) {
        Ok(Some(resolved)) => resolved,
        Ok(None) => return misdirected(),
        Err(_) => return entry_unavailable(&langs, format),
    };
    // 入口容器的标记：恰好一个，取值和主机名的形式对得上。放在标签解析之后：不是插件的名字先按
    // 421 拒（和联邦用例对非服务器名的断言一致），是插件的名字才看标记。
    if !entry_header_matches(&req, kind) {
        return deny(
            StatusCode::BAD_REQUEST,
            "invalid_input",
            "gateway-entry-header-invalid",
            &langs,
            format,
        );
    };
    if !resolved.switches.of(kind) || (kind == Kind::Public && !gateway.public_access_enabled()) {
        return pages::error(
            StatusCode::FORBIDDEN,
            "entry_closed",
            &l10n::text(&langs, "gateway-entry-closed", &[]),
            &langs,
            format,
        );
    }
    let Some(route) = gateway.plugins.route(&resolved.id.slug(), req.uri().path()) else {
        return misdirected();
    };
    let Some(clients) = &route.clients else {
        // 有标签不代表有 HTTP 入口：只声明 `clients.tcp` 或根本没声明 `clients` 的插件不通网页。
        return misdirected();
    };
    let Some(upstream) = &clients.upstream else {
        return plugin_unavailable(&route, &langs, format);
    };
    // 路径按值存：原始路径一路转发，命中 auth_failures 时还要拿它比对。
    let path = req.uri().path().to_owned();
    // 点段在两种入口都拒（ADR-0021「路径怎么匹配」对声明了 public/auth_failures 的入口一致收紧，
    // 局域网的失败计数同样不能被点段绕过）；`public.allow/deny` 仍只限制公网。
    if path_rules::check_dot_segments(&path).is_err() {
        return deny(
            StatusCode::BAD_REQUEST,
            "invalid_input",
            "gateway-invalid-path",
            &langs,
            format,
        );
    }
    if kind == Kind::Public
        && let Some(response) = public_rules_reject(clients, &path, &langs, format)
    {
        return response;
    }
    let entry_cancel = proxy::Cancel::Entry(resolved.revoked.clone());
    let mut req = req;
    let pass = tokio::select! {
        biased;
        _ = entry_cancel.cancelled() => return entry_closed(&langs, format),
        result = auth_reject(gateway, &route, clients, &mut req, &langs, format) => match result {
            Ok(pass) => pass,
            Err(response) => return response,
        },
    };
    let cancel = pass
        .as_ref()
        .map_or(proxy::Cancel::Entry(resolved.revoked.clone()), |pass| {
            proxy::Cancel::Both {
                entry: resolved.revoked.clone(),
                password: pass.revoke.clone(),
            }
        });
    strip_entry_headers(req.headers_mut(), route.proxy.as_deref());
    // 代加的身份头在剥头之后注入：插件声明的同名头已删，这里是唯一来源（ADR-0021「接口」）。
    if let Some(pass) = &pass {
        let headers = req.headers_mut();
        for (name, value) in &pass.headers {
            headers.insert(name.clone(), value.clone());
        }
    }
    forward_to_plugin(
        gateway, req, &route, clients, upstream, cancel, &path, &langs, format,
    )
    .await
}

/// 验证已通过：剥头、代加身份、转发并按入口收尾（Cookie 清洗、失败计数）。
#[allow(clippy::too_many_arguments)]
async fn forward_to_plugin(
    gateway: &Gateway,
    mut req: Request,
    route: &crate::plugin::manager::Route,
    clients: &crate::plugin::manager::ClientsRoute,
    upstream: &crate::plugin::upstreams::Endpoint,
    cancel: proxy::Cancel,
    path: &str,
    langs: &Languages,
    format: pages::Format,
) -> Response {
    // 入口容器写的真实来源；解析不出就不写，不编一个。
    let client = req
        .headers()
        .get("x-real-ip")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<IpAddr>().ok())
        .map(|ip| SocketAddr::new(ip, 0));
    // 关开关断的是这个入口已有的连接：守卫从这进转发层，已升级的连接立即断。
    req.extensions_mut().insert(cancel);
    let forwarded = proxy::Forwarded {
        client,
        scheme: "https",
    };
    let target = proxy::Target {
        endpoint: upstream,
        name: route.id.as_str(),
    };
    let mut response =
        match proxy::forward_or(&gateway.upstream_http, target, req, forwarded, langs).await {
            Ok(response) => response,
            Err(error) => return error.response(route.id.as_str(), langs, format),
        };
    super::isolation::sanitize_set_cookies(response.headers_mut());
    mark_auth_failure(&mut response, clients, path);
    response
}

fn entry_closed(langs: &Languages, format: pages::Format) -> Response {
    pages::error(
        StatusCode::FORBIDDEN,
        "entry_closed",
        &l10n::text(langs, "gateway-entry-closed", &[]),
        langs,
        format,
    )
}

fn entry_unavailable(langs: &Languages, format: pages::Format) -> Response {
    deny(
        StatusCode::SERVICE_UNAVAILABLE,
        "plugin_unavailable",
        "gateway-entry-state-unavailable",
        langs,
        format,
    )
}

/// 入口上的错误页：类别在响应头里，文案四种语言（ADR-0021）。
fn deny(
    status: StatusCode,
    kind: &'static str,
    key: &str,
    langs: &Languages,
    format: pages::Format,
) -> Response {
    pages::error(status, kind, &l10n::text(langs, key, &[]), langs, format)
}

fn plugin_unavailable(route: &Route, langs: &Languages, format: pages::Format) -> Response {
    pages::error(
        StatusCode::SERVICE_UNAVAILABLE,
        "plugin_unavailable",
        &l10n::text(
            langs,
            "gateway-app-not-ready",
            &[("id", route.id.as_str().into())],
        ),
        langs,
        format,
    )
}

/// 公网入口的 `public.allow`/`deny`（局域网入口不受 ADR-0021 的 `public` 限制；点段的检查对
/// 两种入口都在 [`dispatch`] 里执行）。转发给插件的始终是原始路径。
fn public_rules_reject(
    clients: &crate::plugin::manager::ClientsRoute,
    path: &str,
    langs: &Languages,
    format: pages::Format,
) -> Option<Response> {
    let rules =
        clients
            .http
            .public
            .as_ref()
            .and_then(|public| match (&public.allow, &public.deny) {
                (Some(paths), _) => Some(PathRules::Allow(paths.clone())),
                (None, Some(paths)) => Some(PathRules::Deny(paths.clone())),
                (None, None) => None,
            });
    let denied = rules.is_some_and(|rules| !path_rules::public_allows(&rules, path));
    denied.then(|| {
        pages::error(
            StatusCode::FORBIDDEN,
            "public_denied",
            &l10n::text(langs, "gateway-entry-public-denied", &[]),
            langs,
            format,
        )
    })
}

/// `auth` 的验证（ADR-0021「专用密码」、ADR-0022）：`gateway` 用 Basic/Bearer 对专用密码（C16），
/// `credentials` 是 C17 还没实现；`app` 先过 `setup_done`，注册没完成 409，查不出 503。
#[allow(clippy::result_large_err)] // 网关的错误响应直接回给 HTTP 层。
async fn auth_reject(
    gateway: &Gateway,
    route: &Route,
    clients: &crate::plugin::manager::ClientsRoute,
    req: &mut Request,
    langs: &Languages,
    format: pages::Format,
) -> Result<Option<EntryPass>, Response> {
    if clients.http.auth.gateway.is_some() {
        return gateway_authenticate(gateway, route, req, langs, format);
    }
    if clients.http.auth.credentials.is_some() {
        // C17：核心发的专用密码。
        return Err(unsupported(langs, format));
    }
    if clients.http.auth.app.is_none() {
        // 清单校验保证三种恰好一种；都缺的清单到不了运行时，这里不兜底。
        return Err(unsupported(langs, format));
    }
    use crate::plugin::clients_entry::SetupCheck;
    match gateway
        .plugins
        .clients_setup_done(&route.id, clients, gateway.notices.as_ref())
        .await
    {
        SetupCheck::Done => Ok(None),
        SetupCheck::Unavailable => Err(plugin_unavailable(route, langs, format)),
        SetupCheck::Pending => Err(deny(
            StatusCode::CONFLICT,
            "setup_pending",
            "gateway-entry-setup-pending",
            langs,
            format,
        )),
        SetupCheck::Undetermined(stderr) => {
            // 匿名访客只看到通用类别；执行错误（stderr、Docker 错误）写服务器日志（review
            // P0-4：入口的执行错误不能离开已鉴权的管理面）。主人管理接口还没有读它的视图，
            // 日志是目前的完整来源，PR 里写明。
            tracing::error!(plugin = %route.id, "{stderr}");
            Err(plugin_unavailable(route, langs, format))
        }
    }
}

fn unsupported(langs: &Languages, format: pages::Format) -> Response {
    pages::error(
        StatusCode::NOT_IMPLEMENTED,
        "unsupported",
        &l10n::text(langs, "gateway-entry-auth-unsupported", &[]),
        langs,
        format,
    )
}

/// 通过验证后带给转发层的：吊销守卫和按清单模板代加的身份头。
struct EntryPass {
    revoke: watch::Receiver<bool>,
    headers: Vec<(hyper::header::HeaderName, hyper::header::HeaderValue)>,
}

/// `auth: gateway` 的验证（ADR-0021「接口」「防暴力破解」）：只认 Basic（用户名是主人的用户名）
/// 和 Bearer，密码只在核心里比对。没带凭据的质询是 DAV、Git 客户端的第一步，不算验证失败、
/// 不加计数头；带了但不对的 401 带 `X-Wonton-Auth-Failure`，由入口容器计数（没有全局失败上限）。
/// Authorization 在这里删掉，代加的身份头由 [`dispatch`] 在剥头之后注入。
#[allow(clippy::result_large_err)] // 同上：错误响应直接回给 HTTP 层。
fn gateway_authenticate(
    gateway: &Gateway,
    route: &Route,
    req: &mut Request,
    langs: &Languages,
    format: pages::Format,
) -> Result<Option<EntryPass>, Response> {
    let Some(store) = &gateway.plugins.client_passwords else {
        return Err(plugin_unavailable(route, langs, format));
    };
    let username = gateway.plugins.owner_username();
    match gateway_auth::parse_authorization(req.headers(), &username) {
        Attempt::Missing => Err(denied(langs, format, false)),
        Attempt::Invalid => Err(denied(langs, format, true)),
        Attempt::Password(value) => {
            let revoke = match store.authenticate(route.id.as_str(), &value) {
                Ok(Some(revoke)) => revoke,
                Ok(None) => return Err(denied(langs, format, true)),
                Err(error) => {
                    tracing::error!(plugin = %route.id, "读专用密码库失败：{error}");
                    return Err(plugin_unavailable(route, langs, format));
                }
            };
            // 比对通过也删：密码不离开核心，不冒充别的身份。
            req.headers_mut().remove(header::AUTHORIZATION);
            let headers = gateway
                .plugins
                .gateway_auth_headers(&route.id, true)
                .map_err(|error| {
                    tracing::error!(plugin = %route.id, "代加身份头失败：{error}");
                    plugin_unavailable(route, langs, format)
                })?;
            Ok(Some(EntryPass { revoke, headers }))
        }
    }
}

/// 401 + `WWW-Authenticate: Basic`；`counted` 的还给入口容器带计数头。
fn denied(langs: &Languages, format: pages::Format, counted: bool) -> Response {
    let mut response = pages::error(
        StatusCode::UNAUTHORIZED,
        "unauthenticated",
        &l10n::text(langs, "gateway-entry-auth-required", &[]),
        langs,
        format,
    );
    response.headers_mut().insert(
        header::WWW_AUTHENTICATE,
        HeaderValue::from_static("Basic realm=\"wonton\""),
    );
    if counted {
        response
            .headers_mut()
            .insert(AUTH_FAILURE_HEADER, HeaderValue::from_static("1"));
    }
    response
}

/// 请求里的身份和 Wonton 的头一个不留：`webui.proxy` 声明的头（按 C3 的规范化名字比较，`X_Remote_User`
/// 和 `X-Remote-User` 算同一个）和所有 `X-Wonton-*`。这里没有会话，也绝不代登录。
fn strip_entry_headers(headers: &mut HeaderMap, auth: Option<&ProxyAuth>) {
    let wonton: Vec<String> = headers
        .keys()
        .filter(|name| name.as_str().starts_with("x-wonton-"))
        .map(|name| name.as_str().to_owned())
        .collect();
    for name in wonton {
        // http 的 HeaderMap 上 `remove` 删掉这个名字的全部值：留给插件的必须一个不剩。
        headers.remove(name.as_str());
    }
    let Some(auth) = auth else {
        return;
    };
    let declared: Vec<String> = auth
        .header_names()
        .map(|name| name.as_str().to_owned())
        .collect();
    let mut matching: Vec<String> = headers
        .keys()
        .filter(|header| {
            declared
                .iter()
                .any(|name| path_rules::same_header(name, header.as_str()))
        })
        .map(|header| header.as_str().to_owned())
        .collect();
    matching.sort();
    matching.dedup();
    for name in matching {
        headers.remove(name.as_str());
    }
    let cookies: Vec<String> = auth.cookie_names().map(str::to_owned).collect();
    session::strip_cookies(headers, |name| {
        let name = std::str::from_utf8(name).unwrap_or_default();
        cookies
            .iter()
            .any(|declared| path_rules::same_header(declared, name))
    });
}

/// 响应命中 `auth_failures` 声明的路径和状态码就加计数头，入口容器据此计数并删掉；
/// 插件自己带的这个头先删，冒充不了计数（ADR-0021「公网暴露面」）。
fn mark_auth_failure(
    response: &mut Response,
    clients: &crate::plugin::manager::ClientsRoute,
    path: &str,
) {
    let status = response.status().as_u16();
    let headers = response.headers_mut();
    headers.remove(AUTH_FAILURE_HEADER);
    let Some(rules) = &clients.http.auth_failures else {
        return;
    };
    let rules: Vec<path_rules::AuthFailureRule> = rules
        .iter()
        .map(|rule| path_rules::AuthFailureRule {
            path: rule.path.clone(),
            status: rule.status.clone(),
        })
        .collect();
    if path_rules::is_auth_failure(&rules, path, status) {
        headers.insert(AUTH_FAILURE_HEADER, HeaderValue::from_static("1"));
    }
}
