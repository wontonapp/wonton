//! 本地 HTTP 代理：在回环地址上监听，把每条 TCP 连接经隧道转发到服务端。
//!
//! 两种用法见 [`ProxyAccess`]：App 用带会话凭据的 HTTP 正向代理（`http_front` 模块先校验凭据、处理
//! `CONNECT` 和绝对形式的请求行，`Host` 原样保留）；开发工具和测试可以用不校验调用方的反向代理形态，
//! 把 base URL 换成 `http://<local_addr>` 即可。隧道出错时回一个 HTTP 错误响应，用 [`ERROR_HEADER`]
//! 头标明原因。

use std::{io, net::SocketAddr, pin::pin, sync::Arc, time::Duration};

use iroh::endpoint::RecvStream;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{
        TcpListener, TcpStream,
        tcp::{OwnedReadHalf, OwnedWriteHalf},
    },
    task::{JoinHandle, JoinSet},
};
use tracing::{Instrument, debug, debug_span, error, warn};

use crate::{
    client::{TunnelClient, TunnelStream},
    credential::ProxyAccess,
    error::{TunnelError, from_write_error},
    forward::{self, PumpError},
    http_front,
    wire::Request,
};

/// 代理回错误响应时标明原因的头：隧道错误的取值见 [`TunnelError::code`]；代理自己回绝时是
/// `proxy-unauthorized`（缺少或用错会话凭据，407）、`foreign-host`（目标不在家里服务器的域名下，403）
/// 和 `bad-request`（请求头不合法或太大）。
pub const ERROR_HEADER: &str = "x-wonton-tunnel-error";

/// 响应结束后，最多再花这么久收尾客户端没发完的请求，避免空闲写端长期占住流。
const LINGER: Duration = Duration::from_secs(2);

/// accept 出错（例如文件描述符耗尽）后暂停多久再继续，避免空转。
const ACCEPT_BACKOFF: Duration = Duration::from_millis(100);

/// 本地 HTTP 代理。丢弃或调用 [`LocalProxy::shutdown`] 后停止监听，并断开所有经它转发的连接。
#[derive(Debug)]
pub struct LocalProxy {
    local_addr: SocketAddr,
    task: JoinHandle<()>,
}

impl LocalProxy {
    /// 在 `addr` 上开始监听，按 `access` 认调用方。`addr` 必须是回环地址，端口为 0 时由系统分配。
    ///
    /// 只允许回环地址：代理使用本设备的配对身份，监听在局域网上等于把这个身份借给同网段的任何人。
    /// 回环地址同机的其他 App 也连得上，所以 App 要用 [`ProxyAccess::Credential`]。
    pub async fn bind(
        client: TunnelClient,
        addr: SocketAddr,
        access: ProxyAccess,
    ) -> io::Result<Self> {
        if !addr.ip().is_loopback() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("本地代理只能监听回环地址，收到 {addr}"),
            ));
        }
        let listener = TcpListener::bind(addr).await?;
        let local_addr = listener.local_addr()?;
        let task = tokio::spawn(accept_loop(listener, client, Arc::new(access)));
        Ok(Self { local_addr, task })
    }

    /// 实际监听的地址。
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// 停止监听，断开所有经代理转发的连接，并等它们结束。
    pub async fn shutdown(mut self) {
        self.task.abort();
        (&mut self.task).await.ok();
    }
}

impl Drop for LocalProxy {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn accept_loop(listener: TcpListener, client: TunnelClient, access: Arc<ProxyAccess>) {
    let mut connections = JoinSet::new();
    loop {
        tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok((tcp, peer)) => {
                    debug!(%peer, "本地代理收到连接");
                    // 这条连接之后的日志都带上它的本地端口，并发的连接才分得清。
                    let span = debug_span!("本地连接", port = peer.port());
                    connections.spawn(serve_local(client.clone(), tcp, access.clone()).instrument(span));
                }
                Err(err) => {
                    warn!("本地代理接受连接失败：{err}");
                    tokio::time::sleep(ACCEPT_BACKOFF).await;
                }
            },
            // 及时回收结束的连接任务。
            Some(joined) = connections.join_next() => {
                if let Err(err) = joined
                    && err.is_panic()
                {
                    error!("本地代理连接任务 panic：{err}");
                }
            }
        }
    }
}

/// 转发失败的两种情形。
#[derive(Debug)]
enum RelayError {
    /// 服务端还没回任何数据就失败了：可以给本地客户端回一个说明原因的 HTTP 错误。
    BeforeResponse(TunnelError),
    /// 已经开始转发后中断，或者本地客户端一侧出错：只能断开。
    Aborted(PumpError),
}

async fn serve_local(client: TunnelClient, tcp: TcpStream, access: Arc<ProxyAccess>) {
    if let Err(err) = tcp.set_nodelay(true) {
        debug!("本地连接设置 TCP_NODELAY 失败：{err}");
    }
    let (mut tcp_read, mut tcp_write) = tcp.into_split();
    // 正向代理先收下第一个请求头：校验凭据，CONNECT 回 200，绝对形式的请求行改写后送进隧道。
    let prefix = match &*access {
        ProxyAccess::Open => Vec::new(),
        ProxyAccess::Credential { credential, domain } => {
            match http_front::accept(&mut tcp_read, &mut tcp_write, credential, domain).await {
                Some(prefix) => prefix,
                None => return,
            }
        }
    };
    let mut stream = match client.open(Request::Forward).await {
        Ok(stream) => stream,
        Err(err) => return respond_error(&mut tcp_write, &mut tcp_read, &err).await,
    };
    if !prefix.is_empty()
        && let Err(err) = stream.send.write_all(&prefix).await
    {
        stream.abort();
        return respond_error(&mut tcp_write, &mut tcp_read, &from_write_error(&err)).await;
    }
    match relay(&client, &mut stream, &mut tcp_read, &mut tcp_write).await {
        Ok(()) => {}
        Err(RelayError::BeforeResponse(err)) => {
            stream.abort();
            respond_error(&mut tcp_write, &mut tcp_read, &err).await;
        }
        Err(RelayError::Aborted(err)) => {
            debug!("转发中断：{err}");
            stream.abort();
        }
    }
}

/// 双向转发一条本地连接。请求字节不等服务端回状态就开始上传，省掉一个往返。
///
/// 下载方向决定结果：上传方向遇到的隧道错误（例如服务端拒绝后停止接收）由下载方向读到并归类；
/// 只有本地客户端一侧的错误会让上传方向直接结束整条转发。
async fn relay(
    client: &TunnelClient,
    stream: &mut TunnelStream,
    tcp_read: &mut OwnedReadHalf,
    tcp_write: &mut OwnedWriteHalf,
) -> Result<(), RelayError> {
    let TunnelStream {
        send,
        recv,
        conn_id,
    } = stream;
    {
        let mut upload = pin!(forward::tcp_to_tunnel(tcp_read, send));
        let mut download = pin!(download(client, recv, *conn_id, tcp_write));
        let mut upload_done = false;
        loop {
            tokio::select! {
                result = &mut upload, if !upload_done => {
                    upload_done = true;
                    if let Err(PumpError::Tcp(err)) = result {
                        return Err(RelayError::Aborted(PumpError::Tcp(err)));
                    }
                }
                result = &mut download => {
                    result?;
                    if !upload_done {
                        match tokio::time::timeout(LINGER, &mut upload).await {
                            Ok(Ok(())) => {}
                            Ok(Err(err)) => debug!("响应已结束，上传收尾中断：{err}"),
                            Err(_) => debug!("响应已结束，不再等待客户端 EOF"),
                        }
                    }
                    break;
                }
            }
        }
    }
    // 下载已正常结束，不再让迟到的上传错误 reset 响应；取消 pump 后结束剩余发送方向。
    send.finish().ok();
    Ok(())
}

async fn download(
    client: &TunnelClient,
    recv: &mut RecvStream,
    conn_id: usize,
    tcp_write: &mut OwnedWriteHalf,
) -> Result<(), RelayError> {
    client
        .read_status(recv, conn_id)
        .await
        .map_err(RelayError::BeforeResponse)?;
    forward::tunnel_to_tcp(recv, tcp_write)
        .await
        .map_err(RelayError::Aborted)
}

/// 给本地客户端回一个说明隧道错误的 HTTP 响应，然后优雅关闭连接。
async fn respond_error(
    tcp_write: &mut OwnedWriteHalf,
    tcp_read: &mut OwnedReadHalf,
    err: &TunnelError,
) {
    debug!("本地代理回错误响应：{err}");
    respond(tcp_write, tcp_read, &error_response(err)).await;
}

/// 回一个完整的 HTTP 响应（带 `connection: close`），然后优雅关闭连接。
pub(crate) async fn respond(
    tcp_write: &mut OwnedWriteHalf,
    tcp_read: &mut OwnedReadHalf,
    response: &str,
) {
    if let Err(write_err) = tcp_write.write_all(response.as_bytes()).await {
        debug!("回写错误响应失败：{write_err}");
        return;
    }
    tcp_write.shutdown().await.ok();
    linger(tcp_read).await;
}

/// 读掉客户端还没发完的请求再关闭：socket 里留有未读数据时直接关闭，内核会发 RST，客户端可能因此
/// 丢掉已经收到的错误响应。
async fn linger(tcp_read: &mut OwnedReadHalf) {
    let mut buf = vec![0u8; 16 * 1024];
    let drain = async { while matches!(tcp_read.read(&mut buf).await, Ok(n) if n > 0) {} };
    tokio::time::timeout(LINGER, drain).await.ok();
}

fn error_response(err: &TunnelError) -> String {
    let (status, reason) = match err {
        TunnelError::Unauthorized => (403, "Forbidden"),
        TunnelError::UpstreamUnavailable => (503, "Service Unavailable"),
        _ => (502, "Bad Gateway"),
    };
    let body = format!("{err}\n");
    format!(
        "HTTP/1.1 {status} {reason}\r\n\
         {ERROR_HEADER}: {code}\r\n\
         content-type: text/plain; charset=utf-8\r\n\
         content-length: {len}\r\n\
         connection: close\r\n\
         \r\n\
         {body}",
        code = err.code(),
        len = body.len(),
    )
}
