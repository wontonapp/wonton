//! `PairHome`：扫码配对（ADR-0009 的“接口：扫码配对”）。用配对码里的地址连上服务端、出示密钥；服务端登记了
//! 本设备以后，把码里的服务端地址当作连接信息存下，像 `ConnectHome` 一样连上。

use std::sync::Arc;

use serde_json::json;
use wonton_protocol::{CoreError, PairingFailure, PairingStatus};
use wonton_tunnel::TunnelError;
use wonton_tunnel::pair::{self, InvalidCode, PairResponse, PairingCode, RejectReason};

use super::HomeLink;

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
        self.set_pairing(Some(PairingStatus::InProgress));
        self.runtime
            .spawn(self.clone().pair_with(code, device_name.trim().to_owned()));
        Ok(())
    }

    async fn pair_with(self: Arc<Self>, code: PairingCode, device_name: String) {
        let status = match self.exchange(&code, &device_name).await {
            Ok(PairResponse::Paired) => {
                tracing::info!(server = %code.server_id.fmt_short(), "扫码配对成功");
                match self.connect(&ticket(&code)) {
                    Ok(()) => None,
                    Err(e) => Some(failed(PairingFailure::Upstream, e.to_string())),
                }
            }
            Ok(PairResponse::Rejected { reason, message }) => {
                tracing::info!(?reason, "服务端拒绝了配对：{message}");
                Some(failed(rejected(reason), message))
            }
            Err(e) => {
                tracing::info!("配对失败：{e}");
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
        let endpoint = self.endpoint_builder().bind().await.map_err(|e| {
            TunnelError::TransportUnavailable {
                reason: format!("绑定网络失败：{e}"),
            }
        })?;
        let outcome = pair::pair(&endpoint, code, device_name).await;
        endpoint.close().await;
        outcome
    }

    fn set_pairing(&self, status: Option<PairingStatus>) {
        self.view.send_modify(|view| view.pairing = status);
    }
}

/// 存下来的连接信息：码里的服务端地址、基础域名和核心地址，不含配对密钥。
fn ticket(code: &PairingCode) -> String {
    json!({
        "server": {
            "endpoint_id": code.server_id,
            "relay_urls": code.relay_urls,
            "direct_addrs": code.direct_addrs,
        },
        "base_domain": code.base_domain,
        "core_url": code.core_url,
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
