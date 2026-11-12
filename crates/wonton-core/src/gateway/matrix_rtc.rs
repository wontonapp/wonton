//! MatrixRTC 的授权服务入口（ADR-0096）：Matrix 对外地址下的 [`MATRIX_RTC_PREFIX`] 去掉前缀转给
//! `matrix_rtc` 的提供方。Tuwunel 公布的 `livekit_url` 固定指向这里，装卸插件不用改 Tuwunel 的配置。
//!
//! 只过设备门（[`super::matrix::client_request`] 已查过），不要 Matrix 访问令牌：授权靠请求体里的 OpenID 令牌，
//! 由提供方去验。错误按 Matrix 的格式回、带 CORS 头：调用方是插件页里的网页（另一个 origin），读得到原因才能如实提示。

use axum::extract::Request;
use axum::response::Response;
use hyper::header::{ACCESS_CONTROL_ALLOW_ORIGIN, HeaderValue};
use hyper::{StatusCode, Uri};

use super::matrix::{is_ambiguous, matrix_error};
use super::{Gateway, client_addr, proxy};
use crate::plugin::matrix_rtc::{MATRIX_RTC_PREFIX, MatrixRtcUnavailable};

/// 提供方不在：没装、没在运行或连不上。
pub(crate) const KIND_UNAVAILABLE: &str = "matrix_rtc_unavailable";

/// 路径是不是这个前缀下的（`/_wonton/matrix-rtc` 本身或 `/_wonton/matrix-rtc/…`）。
pub(super) fn is_prefixed(path: &str) -> bool {
    path.strip_prefix(MATRIX_RTC_PREFIX)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
}

/// 去掉前缀后的路径和查询：前缀本身对应提供方的 `/`。
fn strip(uri: &Uri) -> Option<String> {
    let rest = uri.path().strip_prefix(MATRIX_RTC_PREFIX)?;
    let path = if rest.is_empty() { "/" } else { rest };
    Some(match uri.query() {
        Some(query) => format!("{path}?{query}"),
        None => path.to_owned(),
    })
}

pub(super) async fn request(gateway: &Gateway, mut req: Request) -> Response {
    let langs = super::api::languages(req.headers());
    if is_ambiguous(req.uri().path()) {
        return with_cors(matrix_error(
            StatusCode::BAD_REQUEST,
            "M_UNRECOGNIZED",
            "invalid_input",
            "路径里不能有 . 或 .. 段、反斜杠，以及编码过的 .、/、\\",
        ));
    }
    let Some(path) = strip(req.uri()).and_then(|path| path.parse::<Uri>().ok()) else {
        return with_cors(matrix_error(
            StatusCode::BAD_REQUEST,
            "M_UNRECOGNIZED",
            "invalid_input",
            "路径不合法",
        ));
    };
    *req.uri_mut() = path;
    let (provider, endpoint) = match gateway.plugins.matrix_rtc_route().await {
        Ok(route) => route,
        Err(MatrixRtcUnavailable::NotInstalled) => {
            return unavailable("没有装提供 matrix_rtc 的插件（通话插件），打不了电话");
        }
        Err(MatrixRtcUnavailable::NotRunning { id, detail }) => {
            return unavailable(&format!("通话插件 {id} 没在运行：{detail}"));
        }
    };
    let name = provider.to_string();
    let target = proxy::Target {
        endpoint: &endpoint,
        name: &name,
    };
    let fwd = proxy::Forwarded {
        client: client_addr(&req),
        scheme: &gateway.public.scheme,
    };
    match proxy::forward_or(&gateway.upstream_http, target, req, fwd, &langs).await {
        Ok(response) => response,
        Err(proxy::ForwardError::HeaderTimeout) => with_cors(matrix_error(
            StatusCode::GATEWAY_TIMEOUT,
            "M_UNKNOWN",
            "upstream_timeout",
            &format!("通话插件 {name} 迟迟没有回应，可以稍后重试"),
        )),
        Err(proxy::ForwardError::Unreachable(e) | proxy::ForwardError::SystemResolution(e)) => {
            unavailable(&format!("通话插件 {name} 连不上：{e}"))
        }
    }
}

fn unavailable(message: &str) -> Response {
    with_cors(matrix_error(
        StatusCode::SERVICE_UNAVAILABLE,
        "M_UNKNOWN",
        KIND_UNAVAILABLE,
        message,
    ))
}

fn with_cors(mut response: Response) -> Response {
    response
        .headers_mut()
        .insert(ACCESS_CONTROL_ALLOW_ORIGIN, HeaderValue::from_static("*"));
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_prefix_and_paths_below_it_match() {
        for path in ["/_wonton/matrix-rtc", "/_wonton/matrix-rtc/sfu/get"] {
            assert!(is_prefixed(path), "{path}");
        }
        for path in [
            "/_wonton/matrix-rtcx",
            "/_wonton/matrix",
            "/_matrix/client/v3/sync",
        ] {
            assert!(!is_prefixed(path), "{path}");
        }
    }

    #[test]
    fn the_prefix_is_stripped_and_the_query_kept() {
        let strip = |uri: &str| strip(&uri.parse::<Uri>().unwrap());
        assert_eq!(strip("/_wonton/matrix-rtc").as_deref(), Some("/"));
        assert_eq!(
            strip("/_wonton/matrix-rtc/sfu/get?a=1").as_deref(),
            Some("/sfu/get?a=1")
        );
    }
}
