//! `PairHome`：扫码配对（ADR-0009 的“接口：扫码配对”）。用配对码里的地址连上服务端、出示密钥；服务端登记了
//! 本设备以后，把码里的服务端地址当作连接信息存下，像 `ConnectHome` 一样连上。

use std::sync::Arc;

use serde_json::json;
use wonton_l10n::l10n;
use wonton_protocol::{CoreError, PairingFailure, PairingStatus};
use wonton_tunnel::TunnelError;
use wonton_tunnel::pair::{self, InvalidCode, PairResponse, PairingCode, RejectReason};

use super::HomeLink;
use crate::l10n::{CATALOG, language};

impl HomeLink {
    pub(crate) fn pair(self: &Arc<Self>, text: &str, device_name: &str) -> Result<(), CoreError> {
        let code = match PairingCode::parse(text) {
            Ok(code) => code,
            Err(e @ InvalidCode::UnsupportedVersion(_)) => {
                self.set_pairing(Some(failed(PairingFailure::Protocol, e.to_string())));
                return Ok(());
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
        self.set_pairing(Some(PairingStatus::InProgress));
        self.runtime
            .spawn(self.clone().pair_with(code, device_name.to_owned()));
        Ok(())
    }

    async fn pair_with(self: Arc<Self>, code: PairingCode, device_name: String) {
        let status = match self.exchange(&code, &device_name).await {
            Ok(PairResponse::Paired) => {
                tracing::info!(server = %code.server_id.fmt_short(), "paired with the home server");
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
                Some(failed(transport(&e), e.to_string()))
            }
        };
        self.set_pairing(status);
    }

    /// 用本设备的身份另开一个 endpoint 配对，配完就关：配上以后由 `connect` 按存下的连接信息建隧道。
    async fn exchange(
        &self,
        code: &PairingCode,
        device_name: &str,
    ) -> Result<PairResponse, TunnelError> {
        // 配对一次成，等不了分档回退：endpoint 放全部档，哪一档能连就从哪一档走（ADR-0048）。
        let endpoint = self
            .endpoint_builder(&code.relays, usize::MAX)
            .bind()
            .await
            .map_err(|e| TunnelError::TransportUnavailable {
                reason: l10n!(
                    CATALOG,
                    language(),
                    "home-pairing-bind-failed",
                    error = e.to_string()
                ),
            })?;
        let outcome = pair::pair(&endpoint, code, device_name).await;
        endpoint.close().await;
        outcome
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
