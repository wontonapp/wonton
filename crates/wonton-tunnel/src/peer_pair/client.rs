//! 发起方：连上对方，交换一条消息和回应。扫码、问结果、送配对结果和删配对通知共用一套收发。

use std::time::Duration;

use iroh::Endpoint;
use iroh::endpoint::{ConnectError, ConnectingError, ConnectionError, RecvStream, VarInt};

use super::{MAX_MESSAGE_LEN, PEER_PAIR_ALPN, PeerIdentity, PeerMessage, PeerPairToken, PeerReply};
use crate::error::{TunnelError, from_connect_error, from_read_error, from_write_error};

/// 建连（包括打洞或经中继）到读完回应的总时间。和设备配对一样，超时按传输不可用处理。
const EXCHANGE_TIMEOUT: Duration = Duration::from_secs(90);

/// TLS 的 `no_application_protocol` 告警：服务端不接受 [`PEER_PAIR_ALPN`]，两端版本不配。
const NO_APPLICATION_PROTOCOL: u64 = 0x100 + 120;

/// 扫对方的码：出示令牌和自己的身份，取回出码方的回应（等主人同意、直接同意，或拒绝的原因）。
pub async fn scan(
    endpoint: &Endpoint,
    code: super::PeerPairingCode,
    identity: PeerIdentity,
) -> Result<PeerReply, TunnelError> {
    let token = code.token.clone();
    let addr = code.server_addr();
    scan_to(endpoint, addr, token, identity).await
}

/// 和 [`scan`] 一样，但直接连已知地址：码里只有中继，同机直连或拿到过对方地址时用它。
pub async fn scan_to(
    endpoint: &Endpoint,
    addr: iroh::EndpointAddr,
    token: PeerPairToken,
    identity: PeerIdentity,
) -> Result<PeerReply, TunnelError> {
    exchange(endpoint, addr, PeerMessage::Scan { token, identity }).await
}

/// 问出码方自己那条请求的结果：等待中、已同意（附出码方的身份）、已拒绝，或者不认识这条请求。
pub async fn ask_outcome(
    endpoint: &Endpoint,
    to: iroh::EndpointAddr,
) -> Result<PeerReply, TunnelError> {
    exchange(endpoint, to, PeerMessage::Outcome).await
}

/// 把主人对一次扫码的决定送给扫码方。回应不是 `Ok`（对方不在等这个结果）时按传输失败处理：
/// 决定没有落地，调用方可以稍后再送。
pub async fn deliver_decision(
    endpoint: &Endpoint,
    to: iroh::EndpointAddr,
    approved: bool,
    identity: PeerIdentity,
) -> Result<(), TunnelError> {
    exchange(endpoint, to, PeerMessage::Decision { approved, identity })
        .await
        .and_then(|reply| match reply {
            PeerReply::Ok => Ok(()),
            PeerReply::Error { message, .. } => Err(TunnelError::transport(format!(
                "对方没有收下配对结果：{message}"
            ))),
            other => Err(TunnelError::protocol(format!(
                "配对结果的回应不对：对方回的是 {other:?}"
            ))),
        })
}

/// 告诉对方我们删了和它的配对。尽力而为：对方收没收到都返回，对方离线时它以后在联邦握手被拒时知道
/// （ADR-0050 的“状态”）。
pub async fn notify_removed(
    endpoint: &Endpoint,
    to: iroh::EndpointAddr,
    server_name: &str,
) -> Result<(), TunnelError> {
    exchange(
        endpoint,
        to,
        PeerMessage::Removed {
            server_name: server_name.to_owned(),
        },
    )
    .await
    .map(|_| ())
}

/// 开一条流，写一条消息，读一条回应，到回应结束（FIN）。
async fn exchange(
    endpoint: &Endpoint,
    addr: iroh::EndpointAddr,
    message: PeerMessage,
) -> Result<PeerReply, TunnelError> {
    let reply = tokio::time::timeout(EXCHANGE_TIMEOUT, exchange_once(endpoint, addr, message))
        .await
        .map_err(|_| {
            TunnelError::transport(format!(
                "{} 秒内没有完成服务器配对的交换",
                EXCHANGE_TIMEOUT.as_secs()
            ))
        })??;
    Ok(reply)
}

async fn exchange_once(
    endpoint: &Endpoint,
    addr: iroh::EndpointAddr,
    message: PeerMessage,
) -> Result<PeerReply, TunnelError> {
    let conn = endpoint
        .connect(addr, PEER_PAIR_ALPN)
        .await
        .map_err(|e| connect_error(&e))?;
    let (mut send, mut recv) = conn.open_bi().await.map_err(TunnelError::transport)?;
    let body = serde_json::to_vec(&message).expect("消息总能编成 JSON");
    send.write_all(&body)
        .await
        .map_err(|e| from_write_error(&e))?;
    send.finish().map_err(TunnelError::transport)?;
    let reply = read_reply(&mut recv).await?;
    conn.close(VarInt::from_u32(0), b"done");
    serde_json::from_slice(&reply)
        .map_err(|e| TunnelError::protocol(format!("服务器配对的回应不合规：{e}")))
}

/// 读回应，直到回应结束（FIN）。
///
/// 服务端总是先写完整回应再关连接（等不到对方先关时由服务端超时关闭，见 server.rs 的 `LINGER`），但
/// FIN 可能没能到达：负载下丢包，或连接先一步关闭、排队中的 FIN 连同回应一起被丢弃。这时只看到连接以
/// `done` 关闭，而已送达的字节正是对方已经做出的决定：能解析出完整回应就按它处理，不把已送达的决定丢成
/// 传输问题；一个字节都没到（对方没来得及回应）或回应不完整时，如实报传输错误。
async fn read_reply(recv: &mut RecvStream) -> Result<Vec<u8>, TunnelError> {
    let mut reply = Vec::new();
    loop {
        // 协议里回应不会接近这个长度；到上限还没读到结束就不再收，与消息长度上限一致。
        if reply.len() >= MAX_MESSAGE_LEN {
            return Err(TunnelError::transport("服务器配对的回应超过长度上限"));
        }
        match recv.read_chunk(MAX_MESSAGE_LEN - reply.len()).await {
            Ok(Some(bytes)) => reply.extend_from_slice(&bytes),
            Ok(None) => return Ok(reply),
            Err(err) => {
                if serde_json::from_slice::<PeerReply>(&reply).is_ok() {
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
        return TunnelError::protocol("对方不接受这个版本的服务器配对协议");
    }
    from_connect_error(err)
}
