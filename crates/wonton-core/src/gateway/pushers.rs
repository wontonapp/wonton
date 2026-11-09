//! 插件网络里的 Bot 只能注册指向家里推送代理的 pusher（ADR-0003 的“接口：推送”）。`matrix.wonton.internal` 上注册
//! pusher 的请求（`POST /_matrix/client/{版本}/pushers/set`）先过这里：HTTP pusher 的地址必须是推送代理，别的种类
//! （比如邮件）都不放行，删除照常放行。
//!
//! Tuwunel 能出网，插件默认不能（插件规范的出网规则）。指向外网的 pusher 会让 Tuwunel 把 Bot 的通知推到外面，不加密
//! 房间里还带着正文，Bot 就绕过了出网限制；Tuwunel 自己的 `ip_range_denylist` 只挡私网地址，挡不住这个。主人经隧道的
//! 设备（`matrix.<base>`）不受限：主人信得过，Element X 这类第三方客户端要注册自己的推送网关（ADR-0015）。

use axum::body::{Body, to_bytes};
use axum::extract::Request;
use axum::response::Response;
use hyper::header::{ACCEPT_LANGUAGE, CONTENT_LENGTH, HeaderValue, TRANSFER_ENCODING};
use hyper::{Method, StatusCode};
use serde_json::Value;

use super::matrix::matrix_error;
use super::push::gateway_url;
use crate::l10n;
use crate::plugin::languages::Languages;

/// 注册 pusher 的请求体上限，正常的只有几百字节。
const MAX_BODY: usize = 64 * 1024;

const KIND_REFUSED: &str = "permission_denied";

/// 是不是注册（或删除）pusher 的请求。
pub(super) fn is_pushers_set(method: &Method, path: &str) -> bool {
    *method == Method::POST
        && path
            .strip_prefix("/_matrix/client/")
            .and_then(|rest| rest.split_once('/'))
            .is_some_and(|(version, rest)| !version.is_empty() && rest == "pushers/set")
}

/// 检查要注册的 pusher；放行时把读出来的请求体原样装回去，不放行时给出要回的响应。
pub(super) async fn check(req: Request) -> Result<Request, Box<Response>> {
    let invalid = |message: &str| {
        Box::new(matrix_error(
            StatusCode::BAD_REQUEST,
            "M_BAD_JSON",
            "invalid_input",
            message,
        ))
    };
    let refused = |message: &str| {
        Box::new(matrix_error(
            StatusCode::FORBIDDEN,
            "M_FORBIDDEN",
            KIND_REFUSED,
            message,
        ))
    };
    let (mut parts, body) = req.into_parts();
    let langs = Languages::from_header(
        parts
            .headers
            .get(ACCEPT_LANGUAGE)
            .and_then(|v| v.to_str().ok()),
    );
    let bytes = to_bytes(body, MAX_BODY).await.map_err(|e| {
        invalid(&l10n::text(
            &langs,
            "gateway-pushers-invalid-body",
            &[("error", e.to_string().into())],
        ))
    })?;
    let request: Value = serde_json::from_slice(&bytes).map_err(|e| {
        invalid(&l10n::text(
            &langs,
            "gateway-pushers-invalid-json",
            &[("error", e.to_string().into())],
        ))
    })?;
    match request.get("kind") {
        // 规范要求 `kind` 必须给出，`null` 表示删除。
        Some(Value::Null) => {}
        Some(Value::String(kind)) if kind == "http" => {
            let url = request.pointer("/data/url").and_then(Value::as_str);
            if url != Some(gateway_url().as_str()) {
                let message = l10n::text(
                    &langs,
                    "gateway-pushers-proxy-only",
                    &[("url", gateway_url().as_str().into())],
                );
                return Err(refused(&message));
            }
        }
        Some(kind) => {
            return Err(refused(&l10n::text(
                &langs,
                "gateway-pushers-kind-refused",
                &[("kind", kind.to_string().into())],
            )));
        }
        None => {
            return Err(invalid(&l10n::text(
                &langs,
                "gateway-pushers-missing-kind",
                &[],
            )));
        }
    }
    // 转发检查过的这一份，不转原文：原文里重复的键 `Value` 只留最后一个，Tuwunel 那边的解析未必一样，两边就会看到
    // 不同的 pusher 地址。
    let checked = serde_json::to_vec(&request).expect("JSON 值总能序列化");
    parts.headers.remove(TRANSFER_ENCODING);
    parts
        .headers
        .insert(CONTENT_LENGTH, HeaderValue::from(checked.len()));
    Ok(Request::from_parts(parts, Body::from(checked)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_pushers_set_endpoint_is_checked() {
        for path in [
            "/_matrix/client/v3/pushers/set",
            "/_matrix/client/r0/pushers/set",
        ] {
            assert!(is_pushers_set(&Method::POST, path), "{path}");
        }
        assert!(!is_pushers_set(
            &Method::GET,
            "/_matrix/client/v3/pushers/set"
        ));
        for path in [
            "/_matrix/client/v3/pushers",
            "/_matrix/client/pushers/set",
            "/_matrix/client//pushers/set",
            "/_matrix/client/v3/x/pushers/set",
        ] {
            assert!(!is_pushers_set(&Method::POST, path), "{path}");
        }
    }
}
