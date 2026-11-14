//! 设备一侧：用配对码连上服务端，出示密钥，取回服务端的回应。

use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use iroh::Endpoint;
use iroh::endpoint::{ConnectError, ConnectingError, ConnectionError, RecvStream, VarInt};
use tracing::debug;

use super::{MAX_MESSAGE_LEN, PAIR_ALPN, PairRequest, PairResponse, PairingCode};
use crate::error::{TunnelError, from_connect_error, from_read_error, from_write_error};

/// 每一小段的时限：建连（包括打洞或经中继）、开一条流、把请求写完、等服务端回应，各算各的。
///
/// 以前整段交换只有一个时限：慢链路上握手和开流就吃掉大半，请求交出去了、回应还在路上就到点，服务端已经登记、
/// 设备这边报失败，留成一个半配对。分开以后一段慢不会占掉另一段的预算。
pub const PAIR_TIMEOUT: Duration = Duration::from_secs(30);

/// TLS 的 `no_application_protocol` 告警：服务端不接受 [`PAIR_ALPN`]，两端版本不配。
const NO_APPLICATION_PROTOCOL: u64 = 0x100 + 120;

/// 用 `code` 连上服务端，请它登记本设备。服务端的拒绝在 `Ok` 里；连不上、协议对不上是 `Err`。
pub async fn pair(
    endpoint: &Endpoint,
    code: &PairingCode,
    device_name: &str,
) -> Result<PairResponse, TunnelError> {
    pair_with_timeout(endpoint, code, device_name, PAIR_TIMEOUT, || {}).await
}

/// 同 [`pair`]，`timeout` 是每一段交换的时限。
///
/// `on_request_sent` 在配对请求整段交出（服务端可能已经开始按它登记）后调用一次，两段交换合计只调一次。
///
/// 开流之后的任何一段超时、或者连接先被对端关掉，都再确认一次：服务端对还在登记表里的同一台设备、同一个码
/// 重发时幂等地回 `Paired`。确认也超时才报失败，拿同一个码手动重试还能配上。
///
/// 建连那一段超时**不**确认，直接报连不上：没有 0-RTT，握手在客户端没完成之前请求一个字节都发不出去，服务端
/// 不可能已经登记；再问一次只会让死服务端的失败从 30 秒拖到 60 秒。开流以后请求就可能已经在路上，那时超时或
/// 连接先关都分不清，一律再问一次。以前只在请求写完之后才确认，开流那一段超时就漏掉了。
pub async fn pair_with_timeout(
    endpoint: &Endpoint,
    code: &PairingCode,
    device_name: &str,
    timeout: Duration,
    on_request_sent: impl Fn() + Send + Sync,
) -> Result<PairResponse, TunnelError> {
    let written = AtomicBool::new(false);
    match exchange(
        endpoint,
        code,
        device_name,
        timeout,
        &written,
        &on_request_sent,
    )
    .await
    {
        Ok(result) => return Ok(result),
        Err(Stop::Failed(error)) => return Err(error),
        // 握手都没完成：请求一个字节都没出去，服务端不可能登记，不再确认。
        Err(Stop::TimedOut("connect")) => {
            debug!("pairing could not connect in time");
            return Err(did_not_finish(timeout));
        }
        Err(Stop::TimedOut(stage)) => {
            debug!(stage, "pairing timed out, confirming with the same code");
        }
        Err(Stop::Unclear(error)) => {
            debug!(
                code = error.code(),
                "pairing lost the connection before the reply, confirming again"
            );
        }
    }
    match exchange(
        endpoint,
        code,
        device_name,
        timeout,
        &written,
        &on_request_sent,
    )
    .await
    {
        Ok(result) => Ok(result),
        Err(Stop::Failed(error)) => Err(error),
        Err(Stop::TimedOut(stage)) => {
            debug!(stage, "the pairing confirmation timed out");
            Err(did_not_finish(timeout))
        }
        Err(Stop::Unclear(error)) => Err(error),
    }
}

/// 一次交换停在哪：可能已经到过服务端（还能再确认一次），或者确定的错误（再问也是一样）。
enum Stop {
    /// 某一段超时。
    TimedOut(&'static str),
    /// 连接在回应到之前被对端关掉：服务端已经登记、回应还在路上时也会这样（它回完就关，`LINGER` 到点也关），
    /// 分不清，按可能已经配上处理。
    Unclear(TunnelError),
    /// 再问也是一样的错误。
    Failed(TunnelError),
}

/// 一小段夹上自己的时限。超时记下是哪一段，方便对照日志。
async fn within<T>(
    timeout: Duration,
    stage: &'static str,
    work: impl Future<Output = Result<T, TunnelError>>,
) -> Result<T, Stop> {
    match tokio::time::timeout(timeout, work).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => Err(Stop::Failed(error)),
        Err(_) => {
            debug!(stage, "pairing stage timed out");
            Err(Stop::TimedOut(stage))
        }
    }
}

fn did_not_finish(timeout: Duration) -> TunnelError {
    TunnelError::transport(format!(
        "pairing did not finish within {}s",
        timeout.as_secs()
    ))
}

async fn exchange(
    endpoint: &Endpoint,
    code: &PairingCode,
    device_name: &str,
    timeout: Duration,
    written: &AtomicBool,
    on_request_sent: &(impl Fn() + Sync),
) -> Result<PairResponse, Stop> {
    let conn = within(timeout, "connect", async {
        endpoint
            .connect(code.server_addr(), PAIR_ALPN)
            .await
            .map_err(|e| connect_error(&e))
    })
    .await?;
    let (mut send, mut recv) = within(timeout, "stream", async {
        conn.open_bi().await.map_err(TunnelError::transport)
    })
    .await?;
    let request = PairRequest::Pair {
        secret: code.secret.clone(),
        device_name: device_name.to_owned(),
    };
    let body = serde_json::to_vec(&request).expect("the request always serializes to JSON");
    within(timeout, "request", async {
        send.write_all(&body)
            .await
            .map_err(|e| from_write_error(&e))?;
        send.finish().map_err(TunnelError::transport)
    })
    .await?;
    // 请求已经全部交出：之后的超时里服务端可能已经按它登记了设备。
    if !written.swap(true, Ordering::Relaxed) {
        on_request_sent();
    }
    // 读回应时连接被关掉：服务端可能已经登记、回应还在路上，先算“不确定”再确认一次。
    let reply = within(timeout, "reply", read_reply(&mut recv))
        .await
        .map_err(|stop| match stop {
            Stop::Failed(error) => Stop::Unclear(error),
            other => other,
        })?;
    conn.close(VarInt::from_u32(0), b"done");
    serde_json::from_slice(&reply)
        .map_err(|e| {
            TunnelError::protocol(format!("malformed pairing response from the server: {e}"))
        })
        .map_err(Stop::Failed)
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
