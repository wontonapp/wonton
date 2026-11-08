//! `PairHome`：扫码配对（ADR-0009 的“接口：扫码配对”）。用配对码里的地址连上服务端、出示密钥；服务端登记了
//! 本设备以后，把码里的服务端地址当作连接信息存下，像 `ConnectHome` 一样连上。

use std::sync::Arc;
use std::time::Instant;

use serde_json::json;
use wonton_l10n::{Lang, l10n};
use wonton_protocol::{CoreError, PairingFailure, PairingStatus};
use wonton_tunnel::TunnelError;
use wonton_tunnel::pair::{self, InvalidCode, PairResponse, PairingCode, RejectReason};

use super::HomeLink;
use crate::l10n::{CATALOG, language};

impl HomeLink {
    /// Validate before changing server scope so an invalid scan does not leave the active server.
    pub(crate) fn validate_pairing(
        &self,
        text: &str,
        device_name: &str,
    ) -> Result<Option<PairingCode>, CoreError> {
        let code = match PairingCode::parse(text) {
            Ok(code) => code,
            Err(e @ InvalidCode::UnsupportedVersion(_)) => {
                self.set_pairing(Some(failed(PairingFailure::Protocol, e.to_string())));
                return Ok(None);
            }
            Err(e) => {
                return Err(CoreError::InvalidInput {
                    reason: e.to_string(),
                });
            }
        };
        // 名字是设备在主人设备列表里的标识，空名字配上去以后没法认。App 的配对页默认带设备名，只有清空了才会走到这里。
        let device_name = device_name.trim();
        if device_name.is_empty() {
            return Err(CoreError::InvalidInput {
                reason: l10n!(CATALOG, language(), "home-pairing-name-empty"),
            });
        }
        Ok(Some(code))
    }

    pub(crate) fn pair(self: &Arc<Self>, code: PairingCode, device_name: &str) {
        let device_name = device_name.trim();
        self.set_pairing(Some(PairingStatus::InProgress));
        self.runtime.spawn(
            self.clone()
                .pair_with(code, device_name.to_owned(), language()),
        );
    }

    async fn pair_with(self: Arc<Self>, code: PairingCode, device_name: String, lang: Lang) {
        let started = Instant::now();
        let mut endpoint = None;
        let exchange = async {
            // Pairing cannot wait for tier fallback: try every relay tier (ADR-0048).
            endpoint = Some(
                self.endpoint_builder(&code.relays, usize::MAX)
                    .bind()
                    .await
                    .map_err(|e| TunnelError::TransportUnavailable {
                        reason: l10n!(
                            CATALOG,
                            lang,
                            "home-pairing-bind-failed",
                            error = e.to_string()
                        ),
                    })?,
            );
            pair::pair(
                endpoint.as_ref().expect("pairing endpoint is bound"),
                &code,
                &device_name,
            )
            .await
        };
        let outcome = match tokio::time::timeout(pair::PAIR_TIMEOUT, exchange).await {
            Ok(result) => result,
            Err(_) => Err(TunnelError::TransportUnavailable {
                reason: l10n!(
                    CATALOG,
                    lang,
                    "home-pairing-timeout",
                    seconds = pair::PAIR_TIMEOUT.as_secs()
                ),
            }),
        };
        let status = match outcome {
            Ok(PairResponse::Paired) => {
                tracing::info!(server = %code.server_id.fmt_short(), "paired with the home server");
                if let Some(endpoint) = endpoint.take() {
                    endpoint.close().await;
                }
                match self.connect(&ticket(&code, &device_name)) {
                    Ok(()) => None,
                    Err(e) => Some(failed(PairingFailure::Upstream, e.to_string())),
                }
            }
            Ok(PairResponse::Rejected { reason, message }) => {
                tracing::info!(?reason, "the home server rejected pairing: {message}");
                Some(failed(rejected(reason), message))
            }
            Err(e) => {
                tracing::info!("pairing failed: {e}");
                let message = l10n!(
                    CATALOG,
                    lang,
                    "home-pairing-failed-detail",
                    addresses = format!("{:?}", code.direct_addrs),
                    relays = code
                        .relays
                        .iter()
                        .map(|relay| relay.url.as_str())
                        .collect::<Vec<_>>()
                        .join(", "),
                    milliseconds = started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
                    seconds = pair::PAIR_TIMEOUT.as_secs(),
                    error = e.to_string()
                );
                Some(failed(transport(&e), message))
            }
        };
        self.set_pairing(status);
        // QUIC draining can take seconds after the exchange deadline; it must not delay the visible result.
        if let Some(endpoint) = endpoint {
            endpoint.close().await;
        }
    }

    pub(crate) fn dismiss_pairing_failure(&self) -> Result<(), CoreError> {
        self.view.send_if_modified(|view| {
            if matches!(view.pairing, Some(PairingStatus::Failed { .. })) {
                view.pairing = None;
                true
            } else {
                false
            }
        });
        Ok(())
    }

    fn set_pairing(&self, status: Option<PairingStatus>) {
        self.view.send_modify(|view| view.pairing = status);
    }
}

/// 存下来的连接信息：码里的服务端地址、基础域名和核心地址，加这台设备的名字，不含配对密钥。
fn ticket(code: &PairingCode, device_name: &str) -> String {
    json!({
        "server": {
            "endpoint_id": code.server_id,
            "relays": code.relays,
            "direct_addrs": code.direct_addrs,
        },
        "base_domain": code.base_domain,
        "core_url": code.core_url,
        "device_name": device_name,
    })
    .to_string()
}

fn failed(reason: PairingFailure, message: String) -> PairingStatus {
    PairingStatus::Failed { reason, message }
}

fn rejected(reason: RejectReason) -> PairingFailure {
    match reason {
        RejectReason::Expired => PairingFailure::Expired,
        RejectReason::Closed => PairingFailure::Closed,
        RejectReason::BadRequest => PairingFailure::Protocol,
        RejectReason::ServerError => PairingFailure::Upstream,
        RejectReason::UnknownCode | RejectReason::Used | RejectReason::Other => {
            PairingFailure::Rejected
        }
    }
}

fn transport(err: &TunnelError) -> PairingFailure {
    match err {
        TunnelError::Protocol { .. } | TunnelError::Unauthorized => PairingFailure::Protocol,
        _ => PairingFailure::TransportUnavailable,
    }
}
