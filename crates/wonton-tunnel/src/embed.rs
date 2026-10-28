//! 把隧道流交给嵌入方：核心在进程内用自己的 HTTP 服务处理每条流，并知道是哪台设备发来的。

use std::{
    io,
    pin::Pin,
    task::{Context, Poll},
};

use iroh::{
    EndpointId,
    endpoint::{RecvStream, SendStream},
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    sync::mpsc,
};

use crate::wire::STREAM_ABORTED;

/// 嵌入方还没取走的流最多排这么多条，再多时新流等待，不会无限堆积。
pub(crate) const BACKLOG: usize = 64;

/// 嵌入方接收隧道流的入口，用法类似 `TcpListener`：每条流对应客户端本地代理收到的一条 TCP 连接。
///
/// 从 [`TunnelProtocol::embedded`](crate::TunnelProtocol::embedded) 或
/// [`TunnelServer::spawn_embedded`](crate::TunnelServer::spawn_embedded) 得到。丢弃它之后，新的流
/// 会以“上游不可用”被拒绝。
#[derive(Debug)]
pub struct TunnelListener {
    streams: mpsc::Receiver<PeerStream>,
}

impl TunnelListener {
    pub(crate) fn channel() -> (mpsc::Sender<PeerStream>, Self) {
        let (sender, streams) = mpsc::channel(BACKLOG);
        (sender, Self { streams })
    }

    /// 等下一条已授权设备的流。隧道服务端关闭后返回 `None`。
    pub async fn accept(&mut self) -> Option<PeerStream> {
        self.streams.recv().await
    }
}

/// 已授权设备的一条隧道流，读写的是客户端本地代理那一侧 TCP 连接的原始字节。
///
/// [`remote_id`](Self::remote_id) 是 QUIC 握手认证过的设备身份，不可伪造。正常结束要调用
/// `shutdown`（tokio 的 `AsyncWriteExt::shutdown`，HTTP 服务库会在连接正常结束时调用）；没有
/// shutdown 就丢弃时复位流，客户端据此知道数据被截断了，而不是正常结束。
#[derive(Debug)]
pub struct PeerStream {
    remote: EndpointId,
    send: SendStream,
    recv: RecvStream,
    finished: bool,
}

impl PeerStream {
    pub(crate) fn new(remote: EndpointId, send: SendStream, recv: RecvStream) -> Self {
        Self {
            remote,
            send,
            recv,
            finished: false,
        }
    }

    /// 对端设备的 endpoint ID。
    pub fn remote_id(&self) -> EndpointId {
        self.remote
    }
}

impl AsyncRead for PeerStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        AsyncRead::poll_read(Pin::new(&mut self.recv), cx, buf)
    }
}

impl AsyncWrite for PeerStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        // SendStream 自己也有一个返回 WriteError 的 poll_write，这里要的是 tokio 的那个。
        AsyncWrite::poll_write(Pin::new(&mut self.send), cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        AsyncWrite::poll_flush(Pin::new(&mut self.send), cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if self.finished {
            return Poll::Ready(Ok(()));
        }
        let result = std::task::ready!(AsyncWrite::poll_shutdown(Pin::new(&mut self.send), cx));
        self.finished = true;
        Poll::Ready(result)
    }
}

impl Drop for PeerStream {
    fn drop(&mut self) {
        if !self.finished {
            self.send.reset(STREAM_ABORTED).ok();
        }
        self.recv.stop(STREAM_ABORTED).ok();
    }
}
