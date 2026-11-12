//! `models.wonton.internal`：插件经核心访问模型网关（插件规范的“模型网关”）。按来源地址认出请求来自哪个插件
//! 网络（见 [`crate::plugin::sources`]），只转发声明了 `models` 的插件的请求，转到网关声明的服务和端口。请求和
//! 响应都流式转发，不缓冲，流式补全的每一块到了就往下送；网关自己的鉴权（凭据）原样带过去。

use axum::extract::Request;
use axum::response::Response;
use hyper::StatusCode;
use hyper::header::ACCEPT_LANGUAGE;

use super::api::error_response;
use super::{Gateway, client_addr, proxy};
use crate::l10n;
use crate::plugin::Permission;
use crate::plugin::languages::Languages;

pub async fn request(gateway: &Gateway, req: Request) -> Response {
    let langs = Languages::from_header(
        req.headers()
            .get(ACCEPT_LANGUAGE)
            .and_then(|v| v.to_str().ok()),
    );
    let source = client_addr(&req).and_then(|addr| gateway.plugins.source(addr.ip()));
    match source {
        Some(source) if source.permissions.contains(&Permission::Models) => {}
        Some(source) => {
            return error_response(
                StatusCode::FORBIDDEN,
                "permission_denied",
                &l10n::text(
                    &langs,
                    "gateway-models-permission",
                    &[("id", source.id.to_string().into())],
                ),
            );
        }
        None => {
            return error_response(
                StatusCode::FORBIDDEN,
                "unauthenticated",
                &l10n::text(&langs, "gateway-plugin-network-only", &[]),
            );
        }
    }
    let Some(route) = gateway.plugins.models_route() else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "models_unavailable",
            &l10n::text(&langs, "gateway-models-down", &[]),
        );
    };
    let fwd = proxy::Forwarded {
        client: client_addr(&req),
        scheme: "http",
    };
    let endpoint = crate::plugin::upstreams::Endpoint::Tcp(route.upstream.clone());
    let name = l10n::text(&langs, "gateway-name-models", &[]);
    let target = proxy::Target {
        endpoint: &endpoint,
        name: &name,
    };
    match proxy::forward_or(&gateway.upstream_http, target, req, fwd, &langs).await {
        Ok(response) => response,
        Err(e) => forward_error(&langs, e),
    }
}

fn forward_error(langs: &Languages, error: proxy::ForwardError) -> Response {
    let name = l10n::text(langs, "gateway-name-models", &[]);
    match error {
        proxy::ForwardError::Unreachable(e) | proxy::ForwardError::SystemResolution(e) => {
            error_response(
                StatusCode::BAD_GATEWAY,
                "upstream_unreachable",
                &l10n::text(
                    langs,
                    "gateway-upstream-unreachable",
                    &[
                        ("name", name.as_str().into()),
                        ("error", e.to_string().into()),
                    ],
                ),
            )
        }
        proxy::ForwardError::HeaderTimeout => error_response(
            StatusCode::GATEWAY_TIMEOUT,
            "upstream_timeout",
            &l10n::text(
                langs,
                "gateway-upstream-timeout",
                &[("name", name.as_str().into())],
            ),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forward_failures_keep_their_own_status_and_kind() {
        for (error, status, kind) in [
            (
                proxy::ForwardError::Unreachable("连不上".into()),
                StatusCode::BAD_GATEWAY,
                "upstream_unreachable",
            ),
            (
                proxy::ForwardError::HeaderTimeout,
                StatusCode::GATEWAY_TIMEOUT,
                "upstream_timeout",
            ),
        ] {
            let response = forward_error(&Languages::default(), error);
            assert_eq!(response.status(), status);
            assert_eq!(response.headers()["x-wonton-error"], kind);
        }
    }
}
