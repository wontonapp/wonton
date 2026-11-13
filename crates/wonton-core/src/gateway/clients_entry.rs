//! 8484 上插件主机名的转发（ADR-0021「核心给入口容器开一个专用监听端口」、ADR-0090）：开关、入口头、
//! 公网路径规则、`setup_done`、头和 Cookie 的清理都在这里。对端是不是入口容器由 `entry_runtime` 的
//! `trusted_entry` 中间件把关；`/_wonton/` 前缀在插件主机名上归核心（ADR-0080），一律 404。

use std::net::{IpAddr, SocketAddr};

use axum::extract::Request;
use axum::response::Response;
use hyper::StatusCode;
use hyper::header::{self, HeaderMap, HeaderValue};

use super::matrix::matrix_error;
use super::{Gateway, api, pages, proxy, session};
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
    if req.uri().path().starts_with("/_wonton/") {
        return pages::error(
            StatusCode::NOT_FOUND,
            "not_found",
            &l10n::text(&langs, "gateway-not-found", &[]),
            &langs,
            format,
        );
    }
    let Some(resolved) = gateway.clients.resolve(&label) else {
        return misdirected();
    };
    // 入口容器的标记：恰好一个，取值和主机名的形式对得上。放在标签解析之后：不是插件的名字先按
    // 421 拒（和联邦用例对非服务器名的断言一致），是插件的名字才看标记。
    if !entry_header_matches(&req, kind) {
        return pages::error(
            StatusCode::BAD_REQUEST,
            "invalid_input",
            &l10n::text(&langs, "gateway-entry-header-invalid", &[]),
            &langs,
            format,
        );
    };
    if !resolved.switches.of(kind) {
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
        let detail = pages::state_of(&route.status, &langs);
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
            format,
        );
    };
    // 路径按值存：原始路径一路转发，命中 auth_failures 时还要拿它比对。
    let path = req.uri().path().to_owned();
    if kind == Kind::Public
        && let Some(response) = public_rules_reject(clients, &path, &langs, format)
    {
        return response;
    }
    if let Some(response) = auth_reject(gateway, &route, clients, &langs, format).await {
        return response;
    }

    let mut req = req;
    strip_entry_headers(req.headers_mut(), route.proxy.as_deref());
    // 入口容器写的真实来源；解析不出就不写，不编一个。
    let client = req
        .headers()
        .get("x-real-ip")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<IpAddr>().ok())
        .map(|ip| SocketAddr::new(ip, 0));
    // 关开关断的是这个入口已有的连接：守卫从这进转发层，已升级的连接立即断。
    req.extensions_mut()
        .insert(proxy::Cancel::Entry(gateway.clients.watch(&label, kind)));
    let forwarded = proxy::Forwarded {
        client,
        scheme: "https",
    };
    let target = proxy::Target {
        endpoint: upstream,
        name: route.id.as_str(),
    };
    let mut response =
        match proxy::forward_or(&gateway.upstream_http, target, req, forwarded, &langs).await {
            Ok(response) => response,
            Err(error) => return error.response(route.id.as_str(), &langs, format),
        };
    mark_auth_failure(&mut response, clients, &path);
    response
}

/// 公网入口的路径规则（局域网入口不受 ADR-0021 的 `public` 限制）：点段一律 400，
/// `public.deny`/`allow` 之外的路径 403。转发给插件的始终是原始路径。
fn public_rules_reject(
    clients: &crate::plugin::manager::ClientsRoute,
    path: &str,
    langs: &Languages,
    format: pages::Format,
) -> Option<Response> {
    if path_rules::check_dot_segments(path).is_err() {
        return Some(pages::error(
            StatusCode::BAD_REQUEST,
            "invalid_input",
            &l10n::text(langs, "gateway-invalid-path", &[]),
            langs,
            format,
        ));
    }
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

/// `auth` 的验证（ADR-0021「专用密码」、ADR-0022）：`gateway` 和 `credentials` 这个 PR 还没实现；
/// `app` 先过 `setup_done`，注册没完成 409，查不出 503。
async fn auth_reject(
    gateway: &Gateway,
    route: &Route,
    clients: &crate::plugin::manager::ClientsRoute,
    langs: &Languages,
    format: pages::Format,
) -> Option<Response> {
    let unsupported = || {
        Some(pages::error(
            StatusCode::NOT_IMPLEMENTED,
            "unsupported",
            &l10n::text(langs, "gateway-entry-auth-unsupported", &[]),
            langs,
            format,
        ))
    };
    if clients.http.auth.gateway.is_some() {
        // C16：网关验证（Basic 和 Bearer）。
        return unsupported();
    }
    if clients.http.auth.credentials.is_some() {
        // C17：核心发的专用密码。
        return unsupported();
    }
    let Some(app) = &clients.http.auth.app else {
        // 清单校验保证三种恰好一种；都缺的清单到不了运行时，这里不兜底。
        return unsupported();
    };
    use crate::plugin::clients_entry::SetupCheck;
    match gateway
        .plugins
        .clients_setup_done(&route.id, &clients.http.service, &app.setup_done)
        .await
    {
        SetupCheck::Done => None,
        SetupCheck::Pending => Some(pages::error(
            StatusCode::CONFLICT,
            "setup_pending",
            &l10n::text(langs, "gateway-entry-setup-pending", &[]),
            langs,
            format,
        )),
        SetupCheck::Undetermined(stderr) => Some(pages::error_with_detail(
            StatusCode::SERVICE_UNAVAILABLE,
            "plugin_unavailable",
            &l10n::text(
                langs,
                "gateway-app-not-ready",
                &[("id", route.id.as_str().into())],
            ),
            Some(&stderr),
            langs,
            format,
        )),
    }
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
