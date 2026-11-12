//! 插件网页的转发：上游地址的选取，以及插件自己网络里的容器访问自己时不带身份地直转（ADR-0063）。

use axum::extract::Request;
use axum::response::Response;
use hyper::header::CONTENT_TYPE;
use hyper::{Method, StatusCode};

use super::{
    Gateway, api, bot_web, client_addr, hosted, isolation, pages, proxy, proxy_sso, request_guards,
    sso, web_storage,
};
use crate::config::PublicUrls;
use crate::l10n;
use crate::plugin::PluginStatus;
use crate::plugin::languages::Languages;
use crate::plugin::upstreams::Endpoint;

/// 回调只借用网关的转发能力；登录请求关联与令牌仍由应用校验（ADR-0078）。
pub(super) fn oidc_callback_post(
    route: &crate::plugin::manager::Route,
    req: &Request,
    public: &PublicUrls,
    origin: &str,
) -> bool {
    req.method() == Method::POST
        && route.oidc_callback
        && req
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(';').next())
            .is_some_and(|media| {
                media
                    .trim()
                    .eq_ignore_ascii_case("application/x-www-form-urlencoded")
            })
        && isolation::oidc_callback_origin_allowed(req.headers(), public, origin)
}

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

/// 插件自己的容器访问自己的网页地址（ADR-0063）：不要会话，直接转给上游。不代表任何人：代登录的
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

pub(super) async fn request(gateway: &Gateway, label: &str, mut req: Request) -> Response {
    let host = gateway.public.plugin_host(label);
    let langs = api::languages(req.headers());
    let format = pages::Format::of(req.headers());
    let bot = match gateway.plugins.bot_web_identity(label) {
        Ok(bot) => bot,
        Err(error) => return bot_web::storage_error(&error, &langs),
    };
    if bot.is_some() || label.starts_with(wonton_protocol::BOT_WEB_LABEL_PREFIX) {
        if let Some(response) = bot_web::source_denied(gateway, &req) {
            return response;
        }
        let Some(bot) = bot else {
            return bot_web::failure(wonton_protocol::BotWebFailure::NotFound, &langs);
        };
        return bot_request(gateway, &bot, &host, req).await;
    }
    let Some(route) = gateway.plugins.route(label) else {
        return pages::error(
            StatusCode::NOT_FOUND,
            "not_installed",
            &crate::l10n::text(
                &langs,
                "gateway-no-app-for-host",
                &[("host", host.as_str().into())],
            ),
            &langs,
            format,
        );
    };
    // 这个插件自己网络里的容器访问它自己的网页地址：不要会话，转给它自己的上游，也不代表任何人（ADR-0063）。
    // 认来源用网关认出调用方的同一张表（按来源地址落在哪个插件网络里）。
    let from_itself = client_addr(&req)
        .and_then(|addr| gateway.plugins.source(addr.ip()))
        .is_some_and(|source| source.id == route.id);
    if from_itself {
        return self_request(gateway, &route, req).await;
    }
    if req.uri().path() == "/_wonton/session" {
        return sso::redeem(gateway, &host, &req);
    }
    let origin = gateway.public.origin(&host);
    let credentials = isolation::credentials_allowed(req.method(), req.headers(), &origin);
    // 登记 OIDC 回调表单（精确路径、urlencoded、来源限签发方/本插件/缺失）无会话可达，身份由应用校验（ADR-0078）。
    let callback = oidc_callback_post(&route, &req, &gateway.public, &origin);
    let person = gateway.plugin_person(&host, &req);
    if !credentials && !callback {
        return request_guards::unauthenticated(gateway, &host, &req, false);
    }
    let person = match person {
        Some(person) => Some(person),
        None if callback => None,
        None => return request_guards::unauthenticated(gateway, &host, &req, true),
    };
    // 插件对家人只看“家人能用”开关：主人开放了才放行（ADR-0045）。核心不查 setup_done（ADR-0021），
    // 家人这一侧没有别的门槛。
    if person.as_ref().is_some_and(|p| !p.is_owner()) && !gateway.plugins.family_enabled(&route.id)
    {
        return pages::error(
            StatusCode::FORBIDDEN,
            "not_shared",
            &l10n::text(
                &langs,
                "gateway-plugin-not-shared",
                &[("id", route.id.as_str().into())],
            ),
            &langs,
            format,
        );
    }
    // 数据存储不经网页的提供方，不看它的状态（ADR-0080 第 6 节）。
    if let Some(hosted) = &route.hosted
        && web_storage::is_data_path(req.uri().path())
    {
        return web_storage::forward(gateway, &route, hosted, req, &origin, &langs, format).await;
    }
    let upstream = match upstream(&route, &langs, format) {
        Ok(upstream) => upstream,
        Err(response) => return *response,
    };
    if let Some(person) = &person
        && let Err(response) = proxy_sso::attach_credentials(
            gateway,
            &route,
            person,
            req.headers_mut(),
            &langs,
            format,
        )
    {
        return *response;
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
    if let Some(auth) = &route.proxy {
        proxy_sso::scrub(response.headers_mut(), auth);
    }
    if route.cross_origin_isolated {
        proxy::add_document_isolation_policy(&mut response);
    }
    response
}

async fn bot_request(
    gateway: &Gateway,
    bot: &wonton_protocol::BotRef,
    host: &str,
    mut req: Request,
) -> Response {
    let langs = api::languages(req.headers());
    if req.uri().path() == "/_wonton/session" {
        return sso::redeem(gateway, host, &req);
    }
    let origin = gateway.public.origin(host);
    if !isolation::credentials_allowed(req.method(), req.headers(), &origin) {
        return bot_web::failure(wonton_protocol::BotWebFailure::Unauthenticated, &langs);
    }
    let person = if let Some(peer) = req.extensions().get().copied() {
        gateway.device_person(Some(peer))
    } else {
        gateway.plugin_person(host, &req)
    };
    if let Err(response) = bot_web::authorize(gateway, bot, person.as_ref(), &langs) {
        return *response;
    }
    let (route, _) = match gateway.plugins.bot_web_target(bot) {
        Ok(target) => target,
        Err(error) => return bot_web::runtime_error(error, &langs),
    };
    let format = pages::Format::of(req.headers());
    let Some(upstream) = route.upstream.as_ref() else {
        return bot_web::failure(wonton_protocol::BotWebFailure::NotReady, &langs);
    };
    if let Some(person) = &person
        && let Err(response) = proxy_sso::attach_credentials(
            gateway,
            &route,
            person,
            req.headers_mut(),
            &langs,
            format,
        )
    {
        return *response;
    }
    let fwd = proxy::Forwarded {
        client: client_addr(&req),
        scheme: &gateway.public.scheme,
    };
    let target = proxy::Target {
        endpoint: upstream,
        name: route.id.as_str(),
    };
    let mut response =
        match proxy::forward_or(&gateway.upstream_http, target, req, fwd, &langs).await {
            Ok(response) => response,
            Err(error) => return error.response(route.id.as_str(), &langs, format),
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
