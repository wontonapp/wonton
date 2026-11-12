//! 插件网页在浏览器里的隔离（ADR-0011“插件隔离的约束”）。
//!
//! 插件子域名彼此同站，也和核心同站，`SameSite=Lax` 拦不住一个插件的页面让浏览器带着 Cookie 向另一个
//! 插件或核心发请求；隧道身份同样随请求自动带上。这里集中四条规则：
//!
//! - 插件子域名上，登录状态只给本插件页面发起的请求和导航：[`credentials_allowed`]；
//!   登记过的 OIDC 回调表单另走受限例外，由应用校验登录请求关联（ADR-0078）。
//! - 核心域名上改状态的请求只接受同源：[`write_allowed`]。
//! - 经隧道调插件管理接口的请求不能是别的网页发起的，读也一样：[`not_from_another_page`]。
//! - 插件响应里的 `Set-Cookie` 不能用网关保留的名字，也不能指定 `Domain`：[`sanitize_set_cookies`]。
//!
//! 浏览器只对 https 和 `localhost` 发 Fetch Metadata（`Sec-Fetch-*`），局域网 http 下只能看 `Origin`。

use hyper::Method;
use hyper::header::{HeaderMap, HeaderValue, ORIGIN, SET_COOKIE};

use super::session::is_reserved_cookie;
use crate::config::PublicUrls;

const SEC_FETCH_SITE: &str = "sec-fetch-site";
const SEC_FETCH_MODE: &str = "sec-fetch-mode";
const SEC_FETCH_DEST: &str = "sec-fetch-dest";

/// 插件子域名上，这个请求能不能带着登录状态到达插件。Cookie 会话、隧道身份，以及 `sso: proxy` 代加的
/// 凭据都按它判断。按 Fetch Metadata 的资源隔离策略，只认这几种请求：
///
/// - `Sec-Fetch-Site` 为 `same-origin`（本插件的页面发起）或 `none`（用户从地址栏、书签打开）；
/// - 导航：`Sec-Fetch-Mode: navigate`，方法为 `GET` 或 `HEAD`；
/// - 没有 `Sec-Fetch-Site` 时，没有 `Origin`，或者 `Origin` 就是 `origin`（本插件的地址）。
///
/// 其余请求按没有登录状态处理；登记的 OIDC 回调 POST 由 `plugin_web::oidc_callback_post` 单独判断。
pub fn credentials_allowed(method: &Method, headers: &HeaderMap, origin: &str) -> bool {
    let site = headers.get(SEC_FETCH_SITE).map(HeaderValue::as_bytes);
    matches!(site, Some(b"same-origin" | b"none"))
        || is_navigation(method, headers)
        || (site.is_none() && origin_absent_or(headers, origin))
}

/// 导航：浏览器打开一个页面（地址栏、链接、跳转、iframe）。`<object>` 和 `<embed>` 的加载也是
/// `navigate` 模式，但内容嵌在发起它的页面里，按资源隔离策略不算。
fn is_navigation(method: &Method, headers: &HeaderMap) -> bool {
    let mode = headers.get(SEC_FETCH_MODE).map(HeaderValue::as_bytes);
    let dest = headers.get(SEC_FETCH_DEST).map(HeaderValue::as_bytes);
    matches!(*method, Method::GET | Method::HEAD)
        && mode == Some(b"navigate")
        && !matches!(dest, Some(b"object" | b"embed"))
}

/// 核心域名上的请求能不能改状态，规则同 Go 的 `http.CrossOriginProtection`：`GET`、`HEAD`、`OPTIONS`
/// 不改状态，放行；其余方法要求 `Sec-Fetch-Site` 为 `same-origin` 或 `none`，没有它时 `Origin` 要么没有、
/// 要么就是 `origin`（核心的地址）。两个头都没有的，是同源的老浏览器或者 curl 这类非浏览器客户端。
pub fn write_allowed(method: &Method, headers: &HeaderMap, origin: &str) -> bool {
    if matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS) {
        return true;
    }
    match headers.get(SEC_FETCH_SITE).map(HeaderValue::as_bytes) {
        Some(site) => matches!(site, b"same-origin" | b"none"),
        None => origin_absent_or(headers, origin),
    }
}

/// 这个请求不是别的网页发起的：`Sec-Fetch-Site` 是 `same-origin` 或 `none`（或者没有），`Origin` 是 `origin`（或者没有）。
/// 客户端核心的请求两个头都不带。
pub fn not_from_another_page(headers: &HeaderMap, origin: &str) -> bool {
    let site = headers.get(SEC_FETCH_SITE).map(HeaderValue::as_bytes);
    matches!(site, None | Some(b"same-origin" | b"none")) && origin_absent_or(headers, origin)
}

/// 没有 `Origin`，或者它就是 `origin`。浏览器总是按 `scheme://host[:port]` 写出 `Origin`（省略默认端口），
/// 和 [`crate::config::PublicUrls::origin`] 的写法一致，所以逐字比较，其他写法一律不算。
fn origin_absent_or(headers: &HeaderMap, origin: &str) -> bool {
    headers
        .get(ORIGIN)
        .is_none_or(|value| value.as_bytes().eq_ignore_ascii_case(origin.as_bytes()))
}

/// 签发方表单与本插件自己的提交；缺 Origin 的命令行也可达，身份不能靠这个头证明。
pub(super) fn oidc_callback_origin_allowed(
    headers: &HeaderMap,
    public: &PublicUrls,
    origin: &str,
) -> bool {
    origin_absent_or(headers, origin)
        || origin_absent_or(headers, &public.origin(&public.auth_host()))
}

/// 过滤插件响应里的 `Set-Cookie`：整条丢弃名字为网关保留（`wonton_*`）的 Cookie，其余的去掉 `Domain`
/// 属性，变成只对插件自己的子域名有效。
///
/// 带 `Domain` 时，浏览器只接受插件自己的域名、基础域名或它的父域，后两种会把 Cookie 发给核心和其他插件。
/// 网关只服务 `<slug>.<base>` 这一个插件域名，去掉 `Domain` 对插件没有区别，所以统一去掉，不去比较域名：
/// 浏览器对 `Domain` 的规范化（大小写、首尾的点、IDNA 映射）很难逐一对齐。
pub fn sanitize_set_cookies(headers: &mut HeaderMap) {
    let original: Vec<HeaderValue> = headers.get_all(SET_COOKIE).iter().cloned().collect();
    if original.is_empty() {
        return;
    }
    headers.remove(SET_COOKIE);
    for value in original.iter().filter_map(|v| sanitize(v.as_bytes())) {
        headers.append(SET_COOKIE, value);
    }
}

/// 按字节处理一条 `Set-Cookie`，其余属性原样保留：Cookie 的值可能不是 ASCII。
fn sanitize(set_cookie: &[u8]) -> Option<HeaderValue> {
    let mut parts = set_cookie.split(|&b| b == b';');
    let pair = parts.next().unwrap_or_default();
    if is_reserved_cookie(sent_name(pair)) {
        return None;
    }
    let mut kept = pair.to_vec();
    for attribute in parts {
        let name = attribute.split(|&b| b == b'=').next().unwrap_or_default();
        if !name.trim_ascii().eq_ignore_ascii_case(b"domain") {
            kept.push(b';');
            kept.extend_from_slice(attribute);
        }
    }
    HeaderValue::from_bytes(&kept).ok()
}

/// 浏览器之后在 `Cookie` 头里写出的名字。没有名字的 Cookie（`Set-Cookie: =wonton_app=x`）只写值，
/// 看上去就是 `wonton_app=x`，所以要看值里第一个 `=` 之前的部分。
pub(super) fn sent_name(pair: &[u8]) -> &[u8] {
    let pair = pair.trim_ascii();
    match split_once(pair, b'=') {
        Some((name, value)) if name.trim_ascii().is_empty() => {
            let value = value.trim_ascii();
            split_once(value, b'=').map_or(value, |(name, _)| name.trim_ascii())
        }
        Some((name, _)) => name.trim_ascii(),
        None => pair,
    }
}

fn split_once(bytes: &[u8], separator: u8) -> Option<(&[u8], &[u8])> {
    let at = bytes.iter().position(|&b| b == separator)?;
    Some((&bytes[..at], &bytes[at + 1..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sanitized(set_cookie: &[u8]) -> Option<Vec<u8>> {
        sanitize(set_cookie).map(|v| v.as_bytes().to_vec())
    }

    #[test]
    fn set_cookie_filter_matches_how_browsers_read_names_and_attributes() {
        // 属性名不分大小写、可以带空白；多个 `Domain` 都去掉；值里的非 ASCII 字节原样保留。
        assert_eq!(
            sanitized(b"a=\xff\xfe; DoMaIn = .wonton.localhost ;Path=/;domain=x; HttpOnly")
                .unwrap(),
            b"a=\xff\xfe;Path=/; HttpOnly"
        );
        assert_eq!(sanitized(b"a=1; Domain").unwrap(), b"a=1");
        // `Domains`、值里出现的 `domain=` 不是 `Domain` 属性。
        assert_eq!(
            sanitized(b"a=domain=x; Domains=y").unwrap(),
            b"a=domain=x; Domains=y"
        );
        for reserved in [
            &b"wonton_app=x"[..],
            b" Wonton_Session=x; Path=/",
            b"=wonton_app=x",
            b" = wonton_app = x",
            b"wonton_app",
        ] {
            assert_eq!(
                sanitized(reserved),
                None,
                "{}",
                String::from_utf8_lossy(reserved)
            );
        }
        assert!(sanitized(b"=x").is_some() && sanitized(b"my_wonton_app=1").is_some());
    }

    #[test]
    fn unknown_fetch_metadata_values_do_not_count_as_same_origin() {
        let own = "http://app-a.wonton.localhost";
        let mut headers = HeaderMap::new();
        headers.insert(
            SEC_FETCH_SITE,
            HeaderValue::from_static("same-origin, none"),
        );
        headers.insert(
            ORIGIN,
            HeaderValue::from_static("http://app-a.wonton.localhost"),
        );
        assert!(!credentials_allowed(&Method::POST, &headers, own));
        assert!(!write_allowed(&Method::POST, &headers, own));
        headers.insert(SEC_FETCH_SITE, HeaderValue::from_static("same-site"));
        headers.insert(SEC_FETCH_MODE, HeaderValue::from_static("navigate"));
        assert!(credentials_allowed(&Method::HEAD, &headers, own));
        assert!(!credentials_allowed(&Method::POST, &headers, own));
    }
}
