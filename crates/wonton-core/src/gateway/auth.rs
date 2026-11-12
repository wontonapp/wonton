//! `auth.<base>` 和插件网络里的 `auth.wonton.internal`：身份提供方（Ory Hydra）的公开端点。这些端点本身就面向
//! 公网设计，由协议保护；真正的用户鉴权发生在核心的登录端点。发现文档在这里按谁来访问改写（ADR-0063）。

use axum::extract::Request;
use axum::response::{IntoResponse, Response};
use hyper::StatusCode;

use super::sharing::{self, SharePeer, oidc};
use super::{Gateway, api, client_addr, pages, proxy};
use crate::l10n;
use crate::plugin::languages::Languages;

pub(super) async fn request(gateway: &Gateway, mut req: Request) -> Response {
    let langs = api::languages(req.headers());
    let format = pages::Format::of(req.headers());
    if let Some(peer) = req.extensions().get::<SharePeer>().cloned() {
        let plugin = match oidc::authorize_auth(gateway, &peer, req.method(), req.uri()) {
            Ok(plugin) => plugin,
            Err(error) => return sharing::refused(error, &langs, format),
        };
        if let Some(plugin) = plugin
            && let Err(error) = sharing::guard_request(gateway, &mut req, &plugin)
        {
            return sharing::refused(error, &langs, format);
        }
        if let Err(error) = oidc::consume_auth(gateway, &peer, req.method(), req.uri()) {
            return sharing::refused(error, &langs, format);
        }
        // Preserve Hydra's code-flow CSRF cookies; challenge handlers bind their subject
        // to SharePeer and never authenticate an owner cookie or bearer token.
        req.headers_mut().remove(hyper::header::AUTHORIZATION);
    }
    if req.uri().path() == crate::oidc::DISCOVERY_PATH {
        return discovery(gateway, req).await;
    }
    let Some(authority) = &gateway.hydra_public else {
        return pages::error(
            StatusCode::SERVICE_UNAVAILABLE,
            "oidc_unavailable",
            &l10n::text(&langs, "gateway-oidc-unconfigured", &[]),
            &langs,
            format,
        );
    };
    let fwd = proxy::Forwarded {
        client: client_addr(&req),
        scheme: &gateway.public.scheme,
    };
    let endpoint = crate::plugin::upstreams::Endpoint::Tcp(authority.clone());
    let name = l10n::text(&langs, "gateway-name-oidc", &[]);
    let target = proxy::Target {
        endpoint: &endpoint,
        name: &name,
    };
    // Keep a guard clone outside the consumed request so revocation also interrupts forwarding.
    let guard = req.extensions().get::<sharing::AccessGuard>().cloned();
    let forward = proxy::forward(&gateway.system_http, target, req, fwd, &langs);
    if let Some(guard) = guard {
        tokio::select! {
            biased;
            _ = guard.cancelled() => sharing::refused(wonton_protocol::GatewayRefusal::ShareRevoked, &langs, format),
            response = forward => response,
        }
    } else {
        forward.await
    }
}

/// 发现文档不原样转发：Hydra 给浏览器和插件后端的端点各错一半，由核心按谁来访问改写
/// （[`crate::oidc::rewrite_discovery`]）。两个主机名拿到同一份改写结果。
async fn discovery(gateway: &Gateway, req: Request) -> Response {
    let langs = api::languages(req.headers());
    let format = pages::Format::of(req.headers());
    let Some(hydra) = &gateway.hydra else {
        return pages::error(
            StatusCode::SERVICE_UNAVAILABLE,
            "oidc_unavailable",
            &l10n::text(&langs, "gateway-oidc-unconfigured", &[]),
            &langs,
            format,
        );
    };
    let browser = gateway.public.origin(&gateway.public.auth_host());
    let backend = gateway.public.auth_internal_origin();
    let mut document = match hydra.discovery().await {
        Ok(document) => document,
        Err(error) => return failure(&error, &langs, format),
    };
    if let Err(error) = crate::oidc::rewrite_discovery(&mut document, &browser, &backend) {
        return failure(&error, &langs, format);
    }
    axum::Json(document).into_response()
}

fn failure(error: &crate::oidc::OidcError, langs: &Languages, format: pages::Format) -> Response {
    let name = l10n::text(langs, "gateway-name-oidc", &[]);
    pages::error(
        StatusCode::BAD_GATEWAY,
        "upstream_unreachable",
        &l10n::text(
            langs,
            "gateway-upstream-unreachable",
            &[("name", name.into()), ("error", error.to_string().into())],
        ),
        langs,
        format,
    )
}
