//! 静态网页的数据存储（ADR-0080 第 6 节）：插件子域名上 `/_wonton/data/` 开头的请求去掉前缀，转给 `web_storage`
//! 的提供方。访问控制和网页一样在这之前做完；写请求另外要同源的 `Origin`，别的网页借着登录状态写不进来。

use axum::extract::Request;
use axum::response::Response;
use hyper::header::{self, HeaderValue};
use hyper::{Method, StatusCode, Uri};

use super::{Gateway, client_addr, hosted, isolation, pages};
use crate::l10n;
use crate::plugin::languages::Languages;
use crate::plugin::manager::{HostedRoute, Route};

/// 数据存储在插件子域名上的前缀。
const PREFIX: &str = "/_wonton/data";

/// 这个路径归数据存储。
pub(super) fn is_data_path(path: &str) -> bool {
    path.strip_prefix(PREFIX)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
}

/// 只读的方法不看 `Origin`：跨站的页面读不到响应（同源策略），也就没什么可防的。其余一律当成写，
/// 包括契约里不支持的方法：不认识的不放松。
fn reads(method: &Method) -> bool {
    matches!(method.as_str(), "GET" | "HEAD" | "OPTIONS" | "PROPFIND")
}

pub(super) async fn forward(
    gateway: &Gateway,
    route: &Route,
    hosted_route: &HostedRoute,
    mut req: Request,
    origin: &str,
    langs: &Languages,
    format: pages::Format,
) -> Response {
    let Some(storage) = &hosted_route.storage else {
        return pages::error(
            StatusCode::NOT_FOUND,
            "not_found",
            &l10n::text(langs, "gateway-web-storage-undeclared", &[]),
            langs,
            format,
        );
    };
    if !reads(req.method()) && !same_origin(req.headers(), origin) {
        return pages::error(
            StatusCode::FORBIDDEN,
            "cross_site",
            &l10n::text(langs, "gateway-web-storage-cross-site", &[]),
            langs,
            format,
        );
    }
    let Some(upstream) = &storage.upstream else {
        return pages::error(
            StatusCode::SERVICE_UNAVAILABLE,
            "not_ready",
            &l10n::text(langs, "gateway-web-storage-not-ready", &[]),
            langs,
            format,
        );
    };
    let Some(uri) = strip(req.uri()) else {
        return pages::error(
            StatusCode::BAD_REQUEST,
            "invalid_path",
            &l10n::text(langs, "gateway-invalid-path", &[]),
            langs,
            format,
        );
    };
    *req.uri_mut() = uri;
    hosted::mark(req.headers_mut(), route.id.as_str());
    let client = client_addr(&req);
    let mut response = match hosted::send(gateway, route, upstream, req, client, langs).await {
        Ok(response) => response,
        Err(error) => return error.response(route.id.as_str(), langs, format),
    };
    isolation::sanitize_set_cookies(response.headers_mut());
    // 存进去的文件也在插件的源上：网页的 CSP 照样盖上，存一个 HTML 再打开它也连不出声明以外的地方。
    hosted::seal(&mut response, hosted_route, route.cross_origin_isolated);
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

/// 写请求来自插件自己的页面：`Origin` 就是插件的源，或者浏览器自己标了 `Sec-Fetch-Site: same-origin`
/// （App 的 WebView 经隧道打开时地址的写法可能和核心认的不完全一样，浏览器的判断不受影响）。两个都没有的
/// 写一律拒绝：别的网页借不到登录状态，curl 这类客户端也不能绕开。
fn same_origin(headers: &hyper::HeaderMap, origin: &str) -> bool {
    headers
        .get(header::ORIGIN)
        .is_some_and(|value| value.as_bytes() == origin.as_bytes())
        || headers
            .get("sec-fetch-site")
            .is_some_and(|value| value.as_bytes() == b"same-origin")
}

/// 去掉前缀，查询串照留：`/_wonton/data/a.json?x` → `/a.json?x`，`/_wonton/data` → `/`。
fn strip(uri: &Uri) -> Option<Uri> {
    let rest = uri.path().strip_prefix(PREFIX)?;
    let path = if rest.is_empty() { "/" } else { rest };
    let path_and_query = match uri.query() {
        Some(query) => format!("{path}?{query}"),
        None => path.to_owned(),
    };
    Uri::builder().path_and_query(path_and_query).build().ok()
}

#[cfg(test)]
mod unit {
    use super::*;

    #[test]
    fn only_the_data_prefix_belongs_to_storage() {
        for path in ["/_wonton/data", "/_wonton/data/", "/_wonton/data/a/b.json"] {
            assert!(is_data_path(path), "{path}");
        }
        for path in [
            "/_wonton/database",
            "/_wonton/datum",
            "/data/a",
            "/_wonton/session",
        ] {
            assert!(!is_data_path(path), "{path}");
        }
    }

    #[test]
    fn the_prefix_is_stripped_and_the_query_kept() {
        let strip = |uri: &str| strip(&uri.parse().unwrap()).unwrap().to_string();
        assert_eq!(strip("/_wonton/data/todo.json?v=1"), "/todo.json?v=1");
        assert_eq!(strip("/_wonton/data"), "/");
        assert_eq!(strip("/_wonton/data/"), "/");
    }
}

#[cfg(test)]
#[path = "web_storage_tests.rs"]
mod tests;
