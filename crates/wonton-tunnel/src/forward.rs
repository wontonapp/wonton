//! TCP 连接与隧道流之间的字节转发，做法参考 dumbpipe 的 `forward_bidi`。
//!
//! 两个方向各自半关闭：TCP 读到 EOF 时 finish 隧道流；隧道流读到结尾时关闭 TCP 写端。任一方向出错时，
//! 调用方用 [`abort`] 复位隧道流，避免对端把被截断的数据当成正常结束（`SendStream` 被丢弃时默认是 finish）。

use std::{fmt, io};

use iroh::endpoint::{RecvStream, SendStream};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::{
    error::{TunnelError, from_read_error, from_write_error},
    wire::STREAM_ABORTED,
};

/// 每次搬运的最大字节数。
const CHUNK: usize = 64 * 1024;

/// 转发在哪一侧失败。
#[derive(Debug)]
pub(crate) enum PumpError {
    /// 本地 TCP 一侧读写失败。
    Tcp(io::Error),
    /// 隧道一侧失败，已归类。
    Tunnel(TunnelError),
}

impl fmt::Display for PumpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Tcp(err) => write!(f, "local TCP connection failed: {err}"),
            Self::Tunnel(err) => err.fmt(f),
        }
    }
}

/// TCP → 隧道，直到 TCP 读到 EOF，然后 finish 隧道流。
pub(crate) async fn tcp_to_tunnel(
    tcp: &mut (impl AsyncRead + Unpin),
    send: &mut SendStream,
) -> Result<(), PumpError> {
    let mut buf = vec![0u8; CHUNK];
    loop {
        let n = tcp.read(&mut buf).await.map_err(PumpError::Tcp)?;
        if n == 0 {
            return send
                .finish()
                .map_err(|err| PumpError::Tunnel(TunnelError::transport(err)));
        }
        send.write_all(&buf[..n])
            .await
            .map_err(|err| PumpError::Tunnel(from_write_error(&err)))?;
    }
}

/// 隧道 → TCP，直到隧道流结束，然后关闭 TCP 写端。
pub(crate) async fn tunnel_to_tcp(
    recv: &mut RecvStream,
    tcp: &mut (impl AsyncWrite + Unpin),
) -> Result<(), PumpError> {
    loop {
        match recv.read_chunk(CHUNK).await {
            Ok(Some(chunk)) => tcp.write_all(&chunk).await.map_err(PumpError::Tcp)?,
            Ok(None) => return tcp.shutdown().await.map_err(PumpError::Tcp),
            Err(err) => return Err(PumpError::Tunnel(from_read_error(&err))),
        }
    }
}

/// 复位隧道流的两个方向。对已经结束的方向是空操作。
pub(crate) fn abort(send: &mut SendStream, recv: &mut RecvStream) {
    send.reset(STREAM_ABORTED).ok();
    recv.stop(STREAM_ABORTED).ok();
}
