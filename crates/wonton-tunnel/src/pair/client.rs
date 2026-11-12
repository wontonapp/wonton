//! 设备一侧：用配对码连上服务端，出示密钥，取回服务端的回应。

use std::time::Duration;

use iroh::Endpoint;
use iroh::endpoint::{ConnectError, ConnectingError, ConnectionError, VarInt};

use super::{MAX_MESSAGE_LEN, PAIR_ALPN, PairRequest, PairResponse, PairingCode};
use crate::error::{TunnelError, from_connect_error, from_write_error};

/// 建连（包括打洞或经中继）到读完回应的总时间。和隧道的请求一样，超时按传输不可用处理。
const PAIR_TIMEOUT: Duration = Duration::from_secs(30);

/// TLS 的 `no_application_protocol` 告警：服务端不接受 [`PAIR_ALPN`]，两端版本不配。
const NO_APPLICATION_PROTOCOL: u64 = 0x100 + 120;

/// 用 `code` 连上服务端，请它登记本设备。服务端的拒绝在 `Ok` 里；连不上、协议对不上是 `Err`。
pub async fn pair(
    endpoint: &Endpoint,
    code: &PairingCode,
    device_name: &str,
) -> Result<PairResponse, TunnelError> {
    tokio::time::timeout(PAIR_TIMEOUT, exchange(endpoint, code, device_name))
        .await
        .map_err(|_| TunnelError::transport(format!("{} 秒内没有配对完", PAIR_TIMEOUT.as_secs())))?
}

async fn exchange(
    endpoint: &Endpoint,
    code: &PairingCode,
    device_name: &str,
) -> Result<PairResponse, TunnelError> {
    let conn = endpoint
        .connect(code.server_addr(), PAIR_ALPN)
        .await
        .map_err(|e| connect_error(&e))?;
    let (mut send, mut recv) = conn.open_bi().await.map_err(TunnelError::transport)?;
    let request = PairRequest::Pair {
        secret: code.secret.clone(),
        device_name: device_name.to_owned(),
    };
    let body = serde_json::to_vec(&request).expect("请求总能编成 JSON");
    send.write_all(&body)
        .await
        .map_err(|e| from_write_error(&e))?;
    send.finish().map_err(TunnelError::transport)?;
    let reply = recv
        .read_to_end(MAX_MESSAGE_LEN)
        .await
        .map_err(TunnelError::transport)?;
    conn.close(VarInt::from_u32(0), b"done");
    serde_json::from_slice(&reply)
        .map_err(|e| TunnelError::protocol(format!("服务端的配对回应不合规：{e}")))
}

fn connect_error(err: &ConnectError) -> TunnelError {
    let closed = match err {
        ConnectError::Connection { source, .. }
        | ConnectError::Connecting {
            source: ConnectingError::ConnectionError { source, .. },
            ..
        } => Some(source),
        _ => None,
    };
    if let Some(ConnectionError::ConnectionClosed(close)) = closed
        && u64::from(close.error_code) == NO_APPLICATION_PROTOCOL
    {
        return TunnelError::protocol("服务端不接受这个版本的配对协议");
    }
    from_connect_error(err)
}
