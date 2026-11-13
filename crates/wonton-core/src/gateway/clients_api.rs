//! 第三方客户端接口的外壳（ADR-0021、ADR-0089、ADR-0090）：只在 `/_wonton/clients` 下挂子模块。
//!
//! 所有请求都要 `X-Wonton-Clients: 1`（插件网页是跨站请求，自定义头要先过预检，核心不回 CORS 头），
//! 而且只认主人：主人令牌，或者主人名下的已配对设备。错误按 [`ClientsFailure`] 的类别回。

use std::sync::Arc;

use axum::Router;
use axum::extract::{Request, State};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use hyper::StatusCode;
use hyper::header::{AUTHORIZATION, CACHE_CONTROL, HeaderValue};
use wonton_protocol::{CLIENTS_ERROR_HEADER, ClientsErrorBody, ClientsFailure, ClientsProblem};

use super::{Gateway, TunnelPeer, api, clients_domain};
use crate::l10n;
use crate::plugin::languages::Languages;

pub fn routes(gateway: Arc<Gateway>) -> Router<Arc<Gateway>> {
    Router::new()
        .route(
            &wonton_protocol::clients_domain_path(),
            get(clients_domain::domain).put(clients_domain::save_domain),
        )
        .route(
            &wonton_protocol::dns_providers_path(),
            get(clients_domain::providers),
        )
        .layer(axum::extract::DefaultBodyLimit::max(16 << 10))
        .layer(middleware::from_fn_with_state(
            gateway,
            owner_with_clients_header,
        ))
}

/// 错误响应：类别进响应头，值不进任何缓存。
pub(super) fn failure(failure: ClientsFailure, message: &str) -> Response {
    let status = failure
        .http_status()
        .and_then(|status| StatusCode::from_u16(status).ok())
        .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let body = ClientsErrorBody {
        error: ClientsProblem {
            kind: failure,
            message: message.to_owned(),
        },
    };
    let mut response = (status, axum::Json(body)).into_response();
    if let Ok(value) = HeaderValue::from_str(failure.as_str()) {
        response.headers_mut().insert(CLIENTS_ERROR_HEADER, value);
    }
    no_store(response)
}

pub(super) fn no_store(mut response: Response) -> Response {
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

pub(super) fn unavailable(langs: &Languages) -> Response {
    failure(
        ClientsFailure::CoreNotInContainer,
        &l10n::text(langs, "clients-unavailable", &[]),
    )
}

async fn owner_with_clients_header(
    State(gateway): State<Arc<Gateway>>,
    request: Request,
    next: Next,
) -> Response {
    let langs = api::languages(request.headers());
    let header_ok = request
        .headers()
        .get(wonton_protocol::CLIENTS_HEADER)
        .and_then(|value| value.to_str().ok())
        == Some("1");
    if !header_ok {
        return failure(
            ClientsFailure::Unauthenticated,
            &l10n::text(&langs, "clients-header-missing", &[]),
        );
    }
    let owner_token = request
        .headers()
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .is_some_and(|token| gateway.check_owner_token(token));
    let owner = owner_token
        || gateway
            .device_person(request.extensions().get::<TunnelPeer>().copied())
            .is_some_and(|person| person.is_owner());
    if !owner {
        return failure(
            ClientsFailure::Unauthenticated,
            &l10n::text(&langs, "clients-unauthenticated", &[]),
        );
    }
    next.run(request).await
}
