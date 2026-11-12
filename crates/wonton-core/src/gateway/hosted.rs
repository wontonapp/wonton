//! 托管插件（ADR-0080）的请求：转给提供方前只留核心写的 `X-Wonton-Plugin`，提供方回 404 时改取清单的
//! `fallback`，回来的响应盖上核心按清单算的 CSP 和 `nosniff`。访问控制在这之前照插件网页做完，提供方不鉴权。

use axum::extract::Request;
use axum::response::Response;
use hyper::header::{self, HeaderName, HeaderValue};
use hyper::{Method, StatusCode, Uri};

use super::{Gateway, client_addr, isolation, pages, proxy};
use crate::plugin::languages::Languages;
use crate::plugin::manager::{HostedRoute, Route};
use crate::plugin::upstreams::Endpoint;

/// 提供方只认这个头选插件的子目录。
pub(crate) const PLUGIN_HEADER: &str = "x-wonton-plugin";

/// 删掉请求里所有 `X-Wonton-*`，再写入插件 ID：页面伪造的头到不了提供方。
pub(crate) fn mark(headers: &mut hyper::HeaderMap, id: &str) {
    let ours: Vec<HeaderName> = headers
        .keys()
        .filter(|name| name.as_str().starts_with("x-wonton-"))
        .cloned()
        .collect();
    for name in ours {
        headers.remove(name);
    }
    let value = HeaderValue::from_str(id).expect("插件 ID 只有小写字母、数字、连字符和点");
    headers.insert(HeaderName::from_static(PLUGIN_HEADER), value);
}

/// 盖上核心的响应头，提供方给的同名头不算数。
pub(crate) fn seal(response: &mut Response, hosted: &HostedRoute, cross_origin_isolated: bool) {
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_str(&hosted.csp).expect("CSP 由校验过的主机名拼成"),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    if cross_origin_isolated {
        proxy::add_document_isolation_policy(response);
    }
}

/// 转发一次；`GET`、`HEAD` 回 404 且清单写了 `fallback` 时，换成 `fallback` 再取一次（单页应用）。
pub(super) async fn forward(
    gateway: &Gateway,
    route: &Route,
    upstream: &Endpoint,
    hosted: &HostedRoute,
    mut req: Request,
    langs: &Languages,
    format: pages::Format,
) -> Response {
    mark(req.headers_mut(), route.id.as_str());
    let retry = hosted
        .fallback
        .as_ref()
        .filter(|_| matches!(*req.method(), Method::GET | Method::HEAD))
        .and_then(|fallback| fallback_request(&req, fallback));
    let client = client_addr(&req);
    let mut response = match send(gateway, route, upstream, req, client, langs).await {
        Ok(response) => response,
        Err(error) => return error.response(route.id.as_str(), langs, format),
    };
    if response.status() == StatusCode::NOT_FOUND
        && let Some(again) = retry
    {
        response = match send(gateway, route, upstream, again, client, langs).await {
            Ok(response) => response,
            Err(error) => return error.response(route.id.as_str(), langs, format),
        };
    }
    isolation::sanitize_set_cookies(response.headers_mut());
    seal(&mut response, hosted, route.cross_origin_isolated);
    response
}

pub(super) async fn send(
    gateway: &Gateway,
    route: &Route,
    upstream: &Endpoint,
    req: Request,
    client: Option<std::net::SocketAddr>,
    langs: &Languages,
) -> Result<Response, proxy::ForwardError> {
    let fwd = proxy::Forwarded {
        client,
        scheme: &gateway.public.scheme,
    };
    let target = proxy::Target {
        endpoint: upstream,
        name: route.id.as_str(),
    };
    proxy::forward_or(&gateway.upstream_http, target, req, fwd, langs).await
}

/// 同样的方法和请求头，路径换成 `fallback`。`GET`、`HEAD` 没有正文，可以照抄。
fn fallback_request(req: &Request, fallback: &str) -> Option<Request> {
    let uri = Uri::builder()
        .path_and_query(format!("/{fallback}"))
        .build()
        .ok()?;
    let mut again = Request::builder()
        .method(req.method().clone())
        .uri(uri)
        .version(req.version())
        .body(axum::body::Body::empty())
        .ok()?;
    // 条件请求和范围请求照抄：这个路径缓存着的本来就是 fallback 的内容。
    *again.headers_mut() = req.headers().clone();
    if let Some(info) = req
        .extensions()
        .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
    {
        again.extensions_mut().insert(*info);
    }
    Some(again)
}

#[cfg(test)]
#[path = "hosted_tests.rs"]
mod tests;
