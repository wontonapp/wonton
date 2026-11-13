use super::{SharePeer, guard_request, now_ms, refused};
use crate::{
    config::HostKind,
    gateway::{Gateway, api, isolation, plugin_web, proxy, proxy_sso},
};
use axum::{Router, extract::Request, response::Response};
use hyper::{Method, StatusCode};
use std::sync::Arc;
use tower::ServiceExt;
use wonton_protocol::GatewayRefusal;

pub(super) async fn dispatch(gateway: Arc<Gateway>, core: Router, req: Request) -> Response {
    let langs = api::languages(req.headers());
    let format = super::super::pages::Format::of(req.headers());
    let peer = req
        .extensions()
        .get::<SharePeer>()
        .expect("private share extension");
    let Some(sharing) = &gateway.sharing else {
        return refused(GatewayRefusal::ShareRequired, &langs, format);
    };
    if let Err(reason) = sharing.person(peer) {
        return refused(reason, &langs, format);
    }
    match gateway.public.classify(&super::super::request_host(&req)) {
        HostKind::Plugin(slug) => plugin(&gateway, &slug, req).await,
        HostKind::Auth => super::super::auth::request(&gateway, req).await,
        HostKind::Core
            if req.method() == Method::GET
                && matches!(
                    req.uri().path(),
                    "/_wonton/oidc/login" | "/_wonton/oidc/consent"
                ) =>
        {
            match core.oneshot(req).await {
                Ok(response) => response,
                Err(never) => match never {},
            }
        }
        _ => refused(GatewayRefusal::ShareRequired, &langs, format),
    }
}

async fn plugin(gateway: &Gateway, slug: &str, mut req: Request) -> Response {
    let langs = api::languages(req.headers());
    let format = super::super::pages::Format::of(req.headers());
    let host = gateway.public.plugin_host(slug);
    // 托管插件（静态网页）第一步不开放给好友（ADR-0080 第 3 节），分享建不出来，这里再拦一道。
    let Some(route) = gateway
        .plugins
        .route(slug, "/")
        .filter(|route| route.hosted.is_none())
    else {
        return refused(GatewayRefusal::ShareRequired, &langs, format);
    };
    // No handoff codes or owner cookies are recognized on the share transport.
    if req.uri().path() == "/_wonton/session" {
        return refused(GatewayRefusal::ShareRequired, &langs, format);
    }
    if !isolation::credentials_allowed(req.method(), req.headers(), &gateway.public.origin(&host)) {
        return refused(GatewayRefusal::ShareRequired, &langs, format);
    }
    if let Err(reason) = guard_request(gateway, &mut req, route.id.as_str()) {
        return refused(reason, &langs, format);
    }
    let peer = req
        .extensions()
        .get::<SharePeer>()
        .expect("private share extension")
        .clone();
    let sharing = gateway.sharing.as_ref().expect("authorized sharing");
    let access = match sharing.authorize(&peer, route.id.as_str(), &gateway.plugins) {
        Ok(access) => access,
        Err(reason) => return refused(reason, &langs, format),
    };
    let upstream = match plugin_web::upstream(&route, &langs, format) {
        Ok(upstream) => upstream,
        Err(response) => return *response,
    };
    let identity = crate::owner::Owner {
        email: Some(crate::owner::friend_email(&access.friend.username)),
        username: access.friend.username,
        display_name: access.friend.display_name.unwrap_or_default(),
    };
    if let Some(auth) = &route.proxy
        && proxy_sso::attach(req.headers_mut(), auth, &identity, false).is_err()
    {
        return super::super::pages::error(
            StatusCode::FORBIDDEN,
            "proxy_credentials",
            "proxy_credentials",
            &langs,
            format,
        );
    }
    let target = proxy::Target {
        endpoint: &upstream,
        name: route.id.as_str(),
    };
    let fwd = proxy::Forwarded {
        client: None,
        scheme: &gateway.public.scheme,
    };
    let mut response =
        match proxy::forward_or(&gateway.upstream_http, target, req, fwd, &langs).await {
            Ok(response) => response,
            Err(error) => return error.response(route.id.as_str(), &langs, format),
        };
    // Only accepted upstream access counts, never authentication or listing. 记不上只是“最近打开过”的
    // 流水账（成百上千个资源各自 fsync 一次），上游已经处理完的请求不能因此被报成授权失效。
    if let Err(error) = sharing.friends.record_opened(&peer.user_id, now_ms()) {
        tracing::warn!(user = %peer.user_id, %error, "记录好友打开时间失败，照常返回上游响应");
    }
    isolation::sanitize_set_cookies(response.headers_mut());
    if let Some(auth) = &route.proxy {
        proxy_sso::scrub(response.headers_mut(), auth);
    }
    if route.cross_origin_isolated {
        proxy::add_document_isolation_policy(&mut response);
    }
    response
}
