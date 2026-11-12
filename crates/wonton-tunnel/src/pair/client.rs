//! 设备一侧：用配对码连上服务端，出示密钥，取回服务端的回应。

use std::time::Duration;

use iroh::Endpoint;
use iroh::endpoint::{ConnectError, ConnectingError, ConnectionError, RecvStream, VarInt};

use super::{MAX_MESSAGE_LEN, PAIR_ALPN, PairRequest, PairResponse, PairingCode};
use crate::error::{TunnelError, from_connect_error, from_read_error, from_write_error};

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
        .map_err(|_| {
            TunnelError::transport(format!(
                "pairing did not finish within {}s",
                PAIR_TIMEOUT.as_secs()
            ))
        })?
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
    let body = serde_json::to_vec(&request).expect("the request always serializes to JSON");
    send.write_all(&body)
        .await
        .map_err(|e| from_write_error(&e))?;
    send.finish().map_err(TunnelError::transport)?;
    let reply = read_reply(&mut recv).await?;
    conn.close(VarInt::from_u32(0), b"done");
    serde_json::from_slice(&reply).map_err(|e| {
        TunnelError::protocol(format!("malformed pairing response from the server: {e}"))
    })
}

/// 读服务端的回应，直到回应结束（FIN）。
///
/// 服务端总是先写完整回应再关连接（等不到设备先关时由服务端超时关闭，见 server.rs 的 `LINGER`），但
/// FIN 可能没能到达设备：负载下丢包，或连接先一步关闭、排队中的 FIN 连同回应一起被丢弃。这时设备拿不到
/// FIN，只看到连接以 `done` 关闭，而已送达的字节正是服务端已经做出的决定：能解析出完整回应就按它处理，
/// 不把已送达的决定丢成传输问题；一个字节都没到（服务端没来得及回应）或回应不完整时，如实报传输错误。
async fn read_reply(recv: &mut RecvStream) -> Result<Vec<u8>, TunnelError> {
    let mut reply = Vec::new();
    loop {
        // 协议里回应不会接近这个长度；到上限还没读到结束就不再收，与回应长度上限一致。
        if reply.len() >= MAX_MESSAGE_LEN {
            return Err(TunnelError::transport(
                "the pairing response from the server exceeds the length limit",
            ));
        }
        match recv.read_chunk(MAX_MESSAGE_LEN - reply.len()).await {
            Ok(Some(bytes)) => reply.extend_from_slice(&bytes),
            Ok(None) => return Ok(reply),
            Err(err) => {
                if serde_json::from_slice::<PairResponse>(&reply).is_ok() {
                    return Ok(reply);
                }
                return Err(from_read_error(&err));
            }
        }
    }
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
        return TunnelError::protocol(
            "the server does not accept this version of the pairing protocol",
        );
    }
    from_connect_error(err)
}
