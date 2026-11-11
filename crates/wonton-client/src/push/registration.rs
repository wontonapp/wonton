//! 经隧道把推送订阅登记给家里的核心（ADR-0003 的“接口：推送”）。请求走本地代理，核心凭隧道身份认出这台设备，
//! 和要 Matrix 登录令牌一样。

use std::time::Duration;

use serde::Deserialize;
use serde_json::json;
use url::Url;
use wonton_l10n::l10n;
use wonton_protocol::{ChatFailure, ProxySettings, PushEndpoint};

use crate::chat::proxy_url;
use crate::l10n::{CATALOG, language};

/// 整个请求最多等这么久，同要登录令牌：隧道卡住时按“传输不可用”失败，回到前台再试。
const TIMEOUT: Duration = Duration::from_secs(30);

const PATH: &str = "/_wonton/push/registration";

/// 核心给的登记结果，字段同核心推送登记接口的响应。
#[derive(Deserialize)]
pub(super) struct Registration {
    pub(super) pushkey: String,
    pub(super) gateway_url: String,
}

/// 核心的错误响应：`{"error": {"kind": …, "message": …}}`。
#[derive(Deserialize)]
struct ErrorBody {
    error: ErrorDetail,
}

#[derive(Deserialize)]
struct ErrorDetail {
    kind: String,
    message: String,
}

/// `PUT <核心地址>/_wonton/push/registration`。
pub(super) async fn register(
    core_url: &Url,
    proxy: &ProxySettings,
    endpoint: &PushEndpoint,
) -> Result<Registration, (ChatFailure, String)> {
    let body = json!({
        "endpoint": endpoint.url,
        "keys": {"p256dh": endpoint.p256dh, "auth": endpoint.auth.expose()},
    });
    let client = client(proxy)?;
    let request = client
        .put(url(core_url))
        .header("content-type", "application/json")
        .body(body.to_string());
    let bytes = send(request, "push-register-failed").await?;
    serde_json::from_slice(&bytes).map_err(|e| {
        (
            ChatFailure::Upstream,
            l10n!(
                CATALOG,
                language(),
                "push-register-invalid-response",
                reason = e.to_string()
            ),
        )
    })
}

/// `DELETE <核心地址>/_wonton/push/registration`。
pub(super) async fn unregister(
    core_url: &Url,
    proxy: &ProxySettings,
) -> Result<(), (ChatFailure, String)> {
    let client = client(proxy)?;
    send(client.delete(url(core_url)), "push-unregister-failed")
        .await
        .map(drop)
}

fn url(core_url: &Url) -> Url {
    core_url.join(PATH).expect("the fixed path always joins")
}

fn client(proxy: &ProxySettings) -> Result<reqwest::Client, (ChatFailure, String)> {
    let proxy = reqwest::Proxy::all(proxy_url(proxy)).map_err(|e| {
        (
            ChatFailure::InvalidInput,
            l10n!(
                CATALOG,
                language(),
                "push-invalid-proxy",
                reason = e.to_string()
            ),
        )
    })?;
    reqwest::Client::builder()
        .proxy(proxy)
        .timeout(TIMEOUT)
        .build()
        .map_err(|e| {
            (
                ChatFailure::Upstream,
                l10n!(
                    CATALOG,
                    language(),
                    "push-http-client",
                    reason = e.to_string()
                ),
            )
        })
}

/// 发请求，成功时返回响应体；失败按原因分类。`failed` 是这次操作的失败整句（`push-register-failed` 等）。
async fn send(
    request: reqwest::RequestBuilder,
    failed: &'static str,
) -> Result<Vec<u8>, (ChatFailure, String)> {
    let transport = |e: reqwest::Error| {
        let reason = if e.is_timeout() {
            l10n!(
                CATALOG,
                language(),
                "push-timeout",
                seconds = TIMEOUT.as_secs()
            )
        } else {
            e.to_string()
        };
        (
            ChatFailure::TransportUnavailable,
            l10n!(CATALOG, language(), failed, reason = reason),
        )
    };
    let response = request.send().await.map_err(transport)?;
    let status = response.status();
    let body = response.bytes().await.map_err(transport)?;
    if status.is_success() {
        return Ok(body.to_vec());
    }
    let detail = serde_json::from_slice::<ErrorBody>(&body)
        .ok()
        .map(|body| body.error);
    let message = detail.as_ref().map_or_else(
        || {
            l10n!(
                CATALOG,
                language(),
                "push-server-status",
                status = status.as_u16()
            )
        },
        |detail| {
            l10n!(
                CATALOG,
                language(),
                "push-server-error",
                message = detail.message.as_str(),
                kind = detail.kind.as_str()
            )
        },
    );
    let reason = match (status.as_u16(), detail.as_ref().map(|d| d.kind.as_str())) {
        (_, Some("push_unavailable")) => ChatFailure::NotReady,
        (401 | 403, _) => ChatFailure::Unauthorized,
        (400, _) => ChatFailure::InvalidInput,
        // 本地代理连不上隧道时回 502，并在 x-wonton-tunnel-error 里写明原因。
        (502, _) => ChatFailure::TransportUnavailable,
        _ => ChatFailure::Upstream,
    };
    Err((reason, l10n!(CATALOG, language(), failed, reason = message)))
}
