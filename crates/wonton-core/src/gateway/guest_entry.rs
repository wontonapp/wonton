//! 访客链接的入口侧（ADR-0090 第 5 节）：`/_wonton/guest/<令牌>` 把它换成只对这个主机名有效的
//! `__Host-` Cookie，其余带有效 Cookie、又在链接声明的路径内的请求按匿名访客放行。
//!
//! 令牌不出现在应用看到的路径里；应用也收不到这个 Cookie（`session::is_reserved_cookie` 在转发层删掉）。
//! 收回和过期都置位链接自己的 watch，已升级的连接由转发层断开。

use axum::extract::Request;
use axum::response::Response;
use hyper::header::{CACHE_CONTROL, HeaderValue, LOCATION, REFERER};
use hyper::{Method, StatusCode};
use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};
use serde::Serialize;

use super::{Gateway, pages, session};
use crate::clients::guests::{Authenticated, COOKIE_NAME};
use crate::entry_haproxy::model::AUTH_FAILURE_HEADER;
use crate::l10n;
use crate::plugin::PluginId;
use crate::plugin::languages::Languages;

/// 核心在插件主机名上给访客链接留的前缀；其余的 `/_wonton/` 仍是一律 404。
pub(super) const PREFIX: &str = "/_wonton/guest/";

/// `Location` 只收可见 ASCII，非 ASCII 的落地路径（如 `/房间`）按 UTF-8 百分号编码。
const LOCATION_ENCODE: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'<')
    .add(b'>')
    .add(b'`')
    .add(b'\\');

/// 换 Cookie 的审计：来源、链接名（ADR-0090 第 5 节）。
#[derive(Serialize)]
struct ExchangeEvent<'a> {
    event: &'static str,
    by: &'static str,
    plugin: &'a str,
    guest: &'a str,
    label: &'a str,
    source: Option<&'a str>,
}

/// 路径正好是 `/_wonton/guest/<令牌>` 时给出令牌。
pub(super) fn token(path: &str) -> Option<&str> {
    let token = path.strip_prefix(PREFIX)?;
    (!token.is_empty() && !token.contains('/')).then_some(token)
}

/// 带访客 Cookie 的请求：Cookie 对上本插件、路径又在链接声明的范围内时才是访客。
pub(super) enum GuestAuth {
    Guest(Authenticated),
    /// 没带、对不上、过期了，或者路径在范围外：按这个入口的正常认证走。
    None,
}

pub(super) fn authenticate(
    gateway: &Gateway,
    id: &PluginId,
    req: &Request,
    path: &str,
) -> GuestAuth {
    let Some(store) = &gateway.plugins.client_guests else {
        return GuestAuth::None;
    };
    for value in session::cookies(req.headers(), COOKIE_NAME) {
        let Ok(guest) = store.authenticate(id.as_str(), value) else {
            continue;
        };
        if crate::clients::path_rules::within(guest.path.as_deref(), path) {
            return GuestAuth::Guest(guest);
        }
    }
    GuestAuth::None
}

/// “链接已失效或不对”（403，入口页面的类别是 `entry_closed`，正文不是 API）。`counted` 的给入口容器
/// 带计数头：错的令牌算一次验证失败。
pub(super) fn invalid(langs: &Languages, format: pages::Format, counted: bool) -> Response {
    let mut response = pages::error(
        StatusCode::FORBIDDEN,
        "entry_closed",
        &l10n::text(langs, "gateway-guest-invalid", &[]),
        langs,
        format,
    );
    if counted {
        response
            .headers_mut()
            .insert(AUTH_FAILURE_HEADER, HeaderValue::from_static("1"));
    }
    response
}

/// 打开链接：校验令牌，换成 Cookie，302 到落地路径。令牌不留在后面的地址里。
pub(super) fn exchange(
    gateway: &Gateway,
    req: Request,
    id: &PluginId,
    langs: &Languages,
    format: pages::Format,
) -> Response {
    let Some(store) = &gateway.plugins.client_guests else {
        return super::clients_entry::unavailable(langs, format);
    };
    if !matches!(*req.method(), Method::GET | Method::HEAD) {
        let mut response = pages::error(
            StatusCode::METHOD_NOT_ALLOWED,
            "invalid_input",
            &l10n::text(langs, "gateway-guest-invalid", &[]),
            langs,
            format,
        );
        response
            .headers_mut()
            .insert(hyper::header::ALLOW, HeaderValue::from_static("GET, HEAD"));
        return response;
    }
    let Some(token) = token(req.uri().path()) else {
        return invalid(langs, format, true);
    };
    let guest = match store.authenticate(id.as_str(), token) {
        Ok(guest) => guest,
        Err(crate::clients::guests::TokenError::Wrong) => return invalid(langs, format, true),
        Err(crate::clients::guests::TokenError::Gone) => return invalid(langs, format, false),
    };
    let remaining =
        std::time::Duration::from_millis(guest.expires_at_ms.saturating_sub(now_ms()).max(1))
            // `Max-Age` 是整秒，不足一秒会写成 0，浏览器当场把刚设的 Cookie 删掉。
            .max(std::time::Duration::from_secs(1));
    let location =
        utf8_percent_encode(guest.path.as_deref().unwrap_or("/"), LOCATION_ENCODE).to_string();
    if let Some(audit) = &gateway.audit {
        audit.record(&ExchangeEvent {
            event: "guest_link_exchange",
            by: "guest",
            plugin: id.as_str(),
            guest: &guest.id,
            label: &guest.label,
            source: req
                .headers()
                .get("x-real-ip")
                .and_then(|value| value.to_str().ok()),
        });
    }
    let mut response = Response::new(axum::body::Body::empty());
    *response.status_mut() = StatusCode::FOUND;
    let headers = response.headers_mut();
    headers.insert(
        LOCATION,
        HeaderValue::from_str(&location).expect("Location 编码后都是可见 ASCII"),
    );
    headers.append(
        hyper::header::SET_COOKIE,
        HeaderValue::from_str(&session::set_cookie(COOKIE_NAME, token, remaining, true))
            .expect("访客 Cookie 都是 ASCII"),
    );
    headers.insert(REFERER, HeaderValue::from_static("no-referrer"));
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    // 这个响应没有应用写的 Cookie，不用过 `sanitize_set_cookies`（它会把我们自己的保留名也删掉）。
    response
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as u64)
}
