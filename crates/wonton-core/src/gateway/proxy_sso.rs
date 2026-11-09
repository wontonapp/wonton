//! 插件网页的网关代登录（`sso: proxy`，插件规范的“网关代登录”）。只在网关认出请求来自哪个已配对设备的人、
//! 请求又带得了登录状态时调用（见 [`super::isolation::credentials_allowed`]），所以凭据只加在这个插件自己的页面发起的请求和导航上，
//! 没登录、跨站和别的插件的请求都碰不到它。
//!
//! - 转发前：去掉浏览器发来的同名请求头和 Cookie（不分大小写），换成声明的值：客户端伪造不了，也覆盖不了。
//! - 转发后：去掉设置这些 Cookie 的 `Set-Cookie`，以及值里带着代加密钥的任何响应头（比如插件把密钥写回
//!   `Set-Cookie` 或跳转地址）。响应体网关不改：插件不应把凭据写进页面。

use hyper::header::{COOKIE, HeaderMap, HeaderName, HeaderValue, SET_COOKIE};

use super::isolation::sent_name;
use super::session::strip_cookies;
use crate::owner::Owner;
use crate::plugin::proxy_auth::ProxyAuth;

/// 转发前：换上代加的请求头和 Cookie；值写不进 HTTP 时报错，不转发。
pub fn attach(headers: &mut HeaderMap, auth: &ProxyAuth, owner: &Owner) -> Result<(), String> {
    for (name, value) in auth.headers(owner) {
        let value = HeaderValue::from_bytes(value.as_bytes())
            .map_err(|_| format!("代加的请求头 {name} 的值写不进 HTTP"))?;
        // 同名的（请求头名不分大小写）全部换掉。
        headers.insert(name, value);
    }
    let cookies = auth.cookies(owner);
    if cookies.is_empty() {
        return Ok(());
    }
    strip_cookies(headers, |name| {
        cookies
            .iter()
            .any(|(declared, _)| declared.as_bytes().eq_ignore_ascii_case(name))
    });
    let mut cookie = headers
        .get(COOKIE)
        .map(|v| v.as_bytes().to_vec())
        .unwrap_or_default();
    for (name, value) in &cookies {
        if !cookie.is_empty() {
            cookie.extend_from_slice(b"; ");
        }
        cookie.extend_from_slice(name.as_bytes());
        cookie.push(b'=');
        cookie.extend_from_slice(value.as_bytes());
    }
    let cookie =
        HeaderValue::from_bytes(&cookie).map_err(|_| "代加的 Cookie 写不进 HTTP".to_owned())?;
    headers.insert(COOKIE, cookie);
    Ok(())
}

/// 转发前：去掉浏览器发来的同名请求头和 Cookie，不加值。插件自己的容器访问自己的网页地址时用（ADR-0063）。
pub fn strip(headers: &mut HeaderMap, auth: &ProxyAuth) {
    for name in auth.header_names() {
        headers.remove(name);
    }
    strip_cookies(headers, |name| {
        auth.cookie_names()
            .any(|declared| declared.as_bytes().eq_ignore_ascii_case(name))
    });
}

/// 转发后：去掉响应里会把代加的凭据交给浏览器的头。
pub fn scrub(headers: &mut HeaderMap, auth: &ProxyAuth) {
    let secrets = auth.secrets();
    let names: Vec<&str> = auth.cookie_names().collect();
    let leaks = |name: &HeaderName, value: &HeaderValue| {
        let bytes = value.as_bytes();
        let sets_ours = *name == SET_COOKIE && {
            let pair = bytes.split(|&b| b == b';').next().unwrap_or_default();
            let sent = sent_name(pair);
            names
                .iter()
                .any(|n| n.as_bytes().eq_ignore_ascii_case(sent))
        };
        sets_ours
            || secrets
                .iter()
                .any(|secret| contains(bytes, secret.as_bytes()))
    };
    let mut kept = HeaderMap::with_capacity(headers.len());
    let mut current: Option<HeaderName> = None;
    for (name, value) in std::mem::take(headers) {
        if let Some(name) = name {
            current = Some(name);
        }
        if let Some(name) = &current
            && !leaks(name, &value)
        {
            kept.append(name.clone(), value);
        }
    }
    *headers = kept;
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}
