//! `models.wonton.internal`：插件经核心访问模型网关（插件规范的“模型网关”）。按来源地址认出请求来自哪个插件
//! 网络（见 [`crate::plugin::sources`]），只转发声明了 `models` 的插件的请求，转到网关声明的服务和端口。请求和
//! 响应都流式转发，不缓冲，流式补全的每一块到了就往下送；网关自己的鉴权（凭据）原样带过去。

use std::pin::Pin;
use std::task::{Context, Poll};

use axum::body::Body;
use axum::extract::Request;
use axum::response::Response;
use hyper::StatusCode;
use hyper::body::{Body as HttpBody, Bytes, Frame, SizeHint};
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
    let source = match source {
        Some(source) if source.permissions.contains(&Permission::Models) => source,
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
    };
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
    let guard = gateway.plugins.activity.start_model(&source.id);
    match proxy::forward_or(&gateway.upstream_http, target, req, fwd, &langs).await {
        Ok(response) => response.map(|inner| {
            let guard = (!inner.is_end_stream()).then_some(guard);
            Body::new(ActivityBody { inner, guard })
        }),
        Err(e) => forward_error(&langs, &route.id, e),
    }
}

struct ActivityBody {
    inner: Body,
    guard: Option<crate::bot_activity::Guard>,
}

impl HttpBody for ActivityBody {
    type Data = Bytes;
    type Error = axum::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Self::Error>>> {
        let frame = std::task::ready!(Pin::new(&mut self.inner).poll_frame(cx));
        if !matches!(&frame, Some(Ok(_))) || self.inner.is_end_stream() {
            self.guard.take();
        }
        Poll::Ready(frame)
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
}

#[cfg(test)]
#[path = "models_activity_tests.rs"]
mod activity_tests;

fn forward_error(
    langs: &Languages,
    route: &crate::plugin::manifest::PluginId,
    error: proxy::ForwardError,
) -> Response {
    let name = route.to_string();
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
            let route = crate::plugin::manifest::PluginId::parse("app.models").unwrap();
            let response = forward_error(&Languages::default(), &route, error);
            assert_eq!(response.status(), status);
            assert_eq!(response.headers()["x-wonton-error"], kind);
        }
    }
}
