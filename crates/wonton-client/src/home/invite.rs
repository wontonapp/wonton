//! `InviteDevice`：已配对的设备经隧道向核心要一个配对码，显示给新设备扫（ADR-0009 的“添加设备”）。请求走本地代理，
//! 核心凭隧道身份认出这台已配对设备，不需要主人令牌。

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Deserialize;
use url::Url;
use wonton_l10n::{Lang, l10n};
use wonton_protocol::{CoreError, DeviceInvite, HomeFailure, ProxySettings, SecretString};

use super::HomeLink;
use super::ticket::Ticket;
use crate::chat::proxy_url;
use crate::l10n::{CATALOG, language};

/// 整个请求（连上、发出、读完回应）最多等这么久，同要 Matrix 登录令牌：隧道卡住时按“连不上”失败，不一直转圈。
pub(crate) const INVITE_TIMEOUT: Duration = Duration::from_secs(30);

/// 核心发码的回应，字段同核心的 `POST /_wonton/api/pairing-codes`。
#[derive(Deserialize)]
struct Issued {
    code: String,
    expires_in: u64,
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

impl HomeLink {
    pub(crate) fn invite(self: &Arc<Self>) -> Result<(), CoreError> {
        let lang = language();
        let Some(text) = self.lock_ticket().clone() else {
            return Err(CoreError::NotReady {
                reason: l10n!(CATALOG, lang, "invite-not-paired"),
            });
        };
        let ticket = Ticket::parse(&text).map_err(|reason| CoreError::Storage {
            reason: l10n!(CATALOG, lang, "invite-invalid-ticket", reason = reason),
        })?;
        let proxy = self.view().proxy;
        self.set_invite(Some(DeviceInvite::Requesting));
        self.runtime
            .spawn(self.clone().request_invite(ticket, proxy, lang));
        Ok(())
    }

    async fn request_invite(
        self: Arc<Self>,
        ticket: Ticket,
        proxy: Option<ProxySettings>,
        lang: Lang,
    ) {
        let issued = match proxy {
            Some(proxy) => issue(&ticket.core_url(), &proxy, ticket.direct_addrs(), lang).await,
            None => Err((
                HomeFailure::TransportUnavailable,
                l10n!(CATALOG, lang, "invite-not-connected"),
            )),
        };
        let invite = match issued {
            Ok(issued) => DeviceInvite::Ready {
                code: SecretString::new(issued.code),
                expires_at_ms: now_ms().saturating_add(issued.expires_in.saturating_mul(1000)),
            },
            Err((reason, message)) => {
                tracing::info!(?reason, "could not get a device pairing code");
                DeviceInvite::Failed { reason, message }
            }
        };
        self.set_invite(Some(invite));
    }

    fn set_invite(&self, invite: Option<DeviceInvite>) {
        self.view.send_modify(|view| view.invite = invite);
    }
}

/// `POST <核心地址>/_wonton/api/pairing-codes`，码里写上本设备连这台服务器用的直连地址：新设备多半和它在同一个网络里。
async fn issue(
    core_url: &Url,
    proxy: &ProxySettings,
    direct_addrs: &[SocketAddr],
    lang: Lang,
) -> Result<Issued, (HomeFailure, String)> {
    let url = core_url
        .join("/_wonton/api/pairing-codes")
        .expect("fixed pairing-code path is valid");
    let client = reqwest::Client::builder()
        .proxy(reqwest::Proxy::all(proxy_url(proxy)).map_err(|e| {
            (
                HomeFailure::Protocol,
                l10n!(
                    CATALOG,
                    lang,
                    "invite-invalid-proxy",
                    reason = e.to_string()
                ),
            )
        })?)
        .timeout(INVITE_TIMEOUT)
        .build()
        .map_err(|e| {
            (
                HomeFailure::Protocol,
                l10n!(CATALOG, lang, "invite-http-client", reason = e.to_string()),
            )
        })?;
    let transport = |e: reqwest::Error| {
        let reason = if e.is_timeout() {
            l10n!(
                CATALOG,
                lang,
                "invite-timeout",
                seconds = INVITE_TIMEOUT.as_secs()
            )
        } else {
            e.to_string()
        };
        (
            HomeFailure::TransportUnavailable,
            l10n!(CATALOG, lang, "invite-request-failed", reason = reason),
        )
    };
    let body = serde_json::json!({ "direct_addrs": direct_addrs }).to_string();
    let response = client
        .post(url)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(body)
        .send()
        .await
        .map_err(transport)?;
    let status = response.status();
    let bytes = response.bytes().await.map_err(transport)?;
    if !status.is_success() {
        return Err(core_error(status.as_u16(), &bytes, lang));
    }
    serde_json::from_slice(&bytes).map_err(|e| {
        (
            HomeFailure::Protocol,
            l10n!(
                CATALOG,
                lang,
                "invite-invalid-response",
                reason = e.to_string()
            ),
        )
    })
}

/// 按核心的回应区分原因。
fn core_error(status: u16, body: &[u8], lang: Lang) -> (HomeFailure, String) {
    let detail = serde_json::from_slice::<ErrorBody>(body)
        .ok()
        .map(|body| body.error);
    let message = detail.as_ref().map_or_else(
        || l10n!(CATALOG, lang, "invite-server-status", status = status),
        |detail| {
            l10n!(
                CATALOG,
                lang,
                "invite-server-error",
                message = detail.message.as_str(),
                kind = detail.kind.as_str()
            )
        },
    );
    let reason = match status {
        // 本设备已被吊销，或者服务器的版本还不让设备经隧道要码。
        401 | 403 => HomeFailure::Unauthorized,
        // 本地代理连不上隧道时回 502，并在 x-wonton-tunnel-error 里写明原因。
        502 => HomeFailure::TransportUnavailable,
        500..=599 => HomeFailure::UpstreamUnavailable,
        _ => HomeFailure::Protocol,
    };
    (
        reason,
        l10n!(CATALOG, lang, "invite-request-failed", reason = message),
    )
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
        })
}
