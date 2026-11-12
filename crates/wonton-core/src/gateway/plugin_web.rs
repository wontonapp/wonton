//! 插件网页的转发：上游地址的选取，以及插件自己网络里的容器访问自己时不带身份地直转（ADR-0063）。

use axum::extract::Request;
use axum::response::Response;
use hyper::StatusCode;

use super::{Gateway, client_addr, isolation, pages, proxy, proxy_sso};
use crate::l10n;
use crate::plugin::PluginStatus;
use crate::plugin::languages::Languages;
use crate::plugin::upstreams::Endpoint;

/// 插件网页的上游地址：运行中直接给；辅助容器坏了但网页还服务着也照常转发，网页里依赖坏掉的
/// 那部分由应用自己报错。没运行时给要直接回给客户端的错误页。
pub(super) fn upstream(
    route: &crate::plugin::manager::Route,
    langs: &Languages,
    format: pages::Format,
) -> Result<Endpoint, Box<Response>> {
    match (&route.status, &route.upstream) {
        (PluginStatus::Running, Some(upstream)) => Ok(upstream.clone()),
        (PluginStatus::Running, None) => Err(Box::new(pages::error(
            StatusCode::NOT_FOUND,
            "no_webui",
            &l10n::text(langs, "gateway-no-webui", &[]),
            langs,
            format,
        ))),
        (PluginStatus::Degraded { .. }, Some(upstream)) if route.web_serving => {
            Ok(upstream.clone())
        }
        (status, _) => {
            let detail = pages::state_of(status, langs);
            Err(Box::new(pages::error_with_detail(
                StatusCode::SERVICE_UNAVAILABLE,
                "not_ready",
                &l10n::text(
                    langs,
                    "gateway-app-not-ready",
                    &[("id", route.id.as_str().into())],
                ),
                Some(&detail),
                langs,
                format,
            )))
        }
    }
}

/// 插件自己的容器访问自己的网页地址（ADR-0063）：不要会话、不查家人开关，直接转给上游。不代表任何人：代登录的
/// 请求头和 Cookie 只剥掉、不注入（来源是真的浏览器请求经插件后端转发时，这些值也属于别人）。
pub(super) async fn self_request(
    gateway: &Gateway,
    route: &crate::plugin::manager::Route,
    mut req: Request,
) -> Response {
    let langs = crate::gateway::api::languages(req.headers());
    let format = pages::Format::of(req.headers());
    let upstream = match upstream(route, &langs, format) {
        Ok(upstream) => upstream,
        Err(response) => return *response,
    };
    if let Some(auth) = &route.proxy {
        proxy_sso::strip(req.headers_mut(), auth);
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
    if route.cross_origin_isolated {
        proxy::add_document_isolation_policy(&mut response);
    }
    response
}
