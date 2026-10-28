//! 隧道服务端：只接受白名单里的设备，把每条流转发到本机的上游 HTTP 服务。

use std::{collections::HashSet, net::SocketAddr};

use iroh::{
    Endpoint, EndpointId,
    endpoint::{Connection, RecvStream, SendStream, VarInt},
    protocol::{AcceptError, ProtocolHandler, Router},
};
use tokio::{net::TcpStream, sync::watch, task::JoinSet};
use tracing::{debug, error, info, warn};

use crate::{
    allowlist::Allowlist,
    forward,
    wire::{ALPN, CLOSE_UNAUTHORIZED, REASON_UNAUTHORIZED, Request, Status},
};

/// 每条连接上客户端可以同时打开的流数。本地代理的每条 TCP 连接占一条流，HTTP keep-alive、长轮询和
/// WebSocket 都会长期占用，所以比 QUIC 默认的 100 放宽。
const MAX_CONCURRENT_STREAMS: VarInt = VarInt::from_u32(256);

/// 隧道服务端协议，挂在 iroh [`Router`] 的 [`ALPN`] 上。
///
/// 独立运行时用 [`TunnelServer`]；核心会把它和配对等其他协议挂在同一个 endpoint 的 router 上。
#[derive(Debug, Clone)]
pub struct TunnelProtocol {
    allowlist: Allowlist,
    upstream: SocketAddr,
}

impl TunnelProtocol {
    /// `upstream` 是隧道流量要转发到的本机 HTTP 服务地址。
    pub fn new(allowlist: Allowlist, upstream: SocketAddr) -> Self {
        Self {
            allowlist,
            upstream,
        }
    }

    /// 接受这条连接上的流，直到连接关闭或设备被吊销。返回结束原因。
    async fn serve(
        &self,
        conn: &Connection,
        remote: EndpointId,
        mut allowlist: watch::Receiver<HashSet<EndpointId>>,
    ) -> String {
        let mut streams = JoinSet::new();
        loop {
            tokio::select! {
                accepted = conn.accept_bi() => match accepted {
                    Ok((send, recv)) => {
                        streams.spawn(serve_stream(send, recv, self.upstream));
                    }
                    Err(err) => return err.to_string(),
                },
                changed = allowlist.changed() => {
                    if changed.is_err() || !allowlist.borrow_and_update().contains(&remote) {
                        conn.close(CLOSE_UNAUTHORIZED, REASON_UNAUTHORIZED);
                        return "设备已被吊销".to_owned();
                    }
                }
                // 及时回收结束的流任务，长连接上的 JoinSet 才不会越积越多。
                Some(joined) = streams.join_next() => {
                    if let Err(err) = joined
                        && err.is_panic()
                    {
                        error!(remote = %remote.fmt_short(), "隧道流任务 panic：{err}");
                    }
                }
            }
        }
    }
}

impl ProtocolHandler for TunnelProtocol {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        let remote = conn.remote_id();
        // 先订阅再检查：检查之后、订阅之前发生的吊销也会被看到。
        let mut allowlist = self.allowlist.subscribe();
        if !allowlist.borrow_and_update().contains(&remote) {
            warn!(remote = %remote, "拒绝未配对的设备");
            conn.close(CLOSE_UNAUTHORIZED, REASON_UNAUTHORIZED);
            return Ok(());
        }
        conn.set_max_concurrent_bi_streams(MAX_CONCURRENT_STREAMS);
        info!(remote = %remote.fmt_short(), "设备已连接");
        let reason = self.serve(&conn, remote, allowlist).await;
        info!(remote = %remote.fmt_short(), %reason, "设备连接结束");
        Ok(())
    }
}

async fn serve_stream(mut send: SendStream, mut recv: RecvStream, upstream: SocketAddr) {
    let mut kind = [0u8; 1];
    if let Err(err) = recv.read_exact(&mut kind).await {
        debug!("流在请求类型之前就结束了：{err}");
        return;
    }
    match Request::from_byte(kind[0]) {
        Some(Request::Forward) => forward_to_upstream(send, recv, upstream).await,
        Some(Request::Ping) => reply(&mut send, Status::Ok).await,
        None => {
            warn!(kind = kind[0], "不支持的请求类型");
            reply(&mut send, Status::UnsupportedRequest).await;
        }
    }
}

async fn forward_to_upstream(mut send: SendStream, mut recv: RecvStream, upstream: SocketAddr) {
    let mut tcp = match TcpStream::connect(upstream).await {
        Ok(tcp) => tcp,
        Err(err) => {
            warn!(%upstream, "连不上上游 HTTP 服务：{err}");
            reply(&mut send, Status::UpstreamUnavailable).await;
            return;
        }
    };
    if let Err(err) = tcp.set_nodelay(true) {
        debug!("上游连接设置 TCP_NODELAY 失败：{err}");
    }
    if let Err(err) = send.write_all(&[Status::Ok as u8]).await {
        debug!("回写状态失败：{err}");
        forward::abort(&mut send, &mut recv);
        return;
    }
    let (mut tcp_read, mut tcp_write) = tcp.split();
    let result = tokio::try_join!(
        forward::tunnel_to_tcp(&mut recv, &mut tcp_write),
        forward::tcp_to_tunnel(&mut tcp_read, &mut send),
    );
    if let Err(err) = result {
        debug!("转发中断：{err}");
        forward::abort(&mut send, &mut recv);
    }
}

async fn reply(send: &mut SendStream, status: Status) {
    match send.write_all(&[status as u8]).await {
        Ok(()) => {
            send.finish().ok();
        }
        Err(err) => debug!("回写状态失败：{err}"),
    }
}

/// 独立运行的隧道服务端：一个 iroh endpoint 加一个只挂了 [`TunnelProtocol`] 的 router。
#[derive(Debug)]
pub struct TunnelServer {
    router: Router,
    allowlist: Allowlist,
}

impl TunnelServer {
    /// 在 `endpoint` 上开始接受隧道连接，只放行 `allowlist` 里的设备，流量转发到 `upstream`。
    pub fn spawn(endpoint: Endpoint, allowlist: Allowlist, upstream: SocketAddr) -> Self {
        let protocol = TunnelProtocol::new(allowlist.clone(), upstream);
        let router = Router::builder(endpoint).accept(ALPN, protocol).spawn();
        Self { router, allowlist }
    }

    /// 服务端的 iroh endpoint。
    pub fn endpoint(&self) -> &Endpoint {
        self.router.endpoint()
    }

    /// 服务端使用的白名单，可在运行时增删设备。
    pub fn allowlist(&self) -> &Allowlist {
        &self.allowlist
    }

    /// 停止接受新连接，关闭现有连接和 endpoint，等对端确认关闭（最多几秒）。
    pub async fn shutdown(self) {
        match self.router.shutdown().await {
            Ok(()) => {}
            Err(err) if err.is_panic() => std::panic::resume_unwind(err.into_panic()),
            Err(err) => error!("隧道服务端关闭异常：{err}"),
        }
    }
}
