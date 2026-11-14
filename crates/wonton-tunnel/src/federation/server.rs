//! 联邦入站：只接受服务器名单里的对方服务器，把每条流原样接到本机 Tuwunel 的联邦端口。
//!
//! 线协议和隧道的一样（见 crate 级文档的“线协议”），但白名单是服务器名单：配对过的服务器进不了
//! App 的隧道，配对过的设备也进不了联邦（ADR-0050 的“名单分开”）。

use std::{
    collections::HashSet,
    sync::atomic::{AtomicU64, Ordering},
};

use iroh::{
    EndpointId,
    endpoint::{Connection, RecvStream, SendStream, VarInt},
    protocol::{AcceptError, ProtocolHandler},
};
use tokio::{net::TcpStream, sync::watch, task::JoinSet};
use tracing::{debug, error, info, warn};

use crate::{
    allowlist::Allowlist,
    error::connection_log_code,
    forward,
    wire::{CLOSE_UNAUTHORIZED, REASON_UNAUTHORIZED, Request, Status},
};

/// 每条连接上对方可以同时打开的流数。Tuwunel 的联邦请求是并发的，keep-alive 和长连接会长期占用，
/// 取值和隧道服务端一样放宽。
const MAX_CONCURRENT_STREAMS: VarInt = VarInt::from_u32(256);

/// 联邦服务端协议，挂在 iroh [`Router`](iroh::protocol::Router) 的
/// [`FEDERATION_ALPN`](crate::federation::FEDERATION_ALPN) 上。
#[derive(Debug)]
pub struct FederationProtocol {
    allowlist: Allowlist,
    /// 本机 Tuwunel 的联邦地址（主机名:端口），入站的流都接到它；按连接现解析，容器重建后地址变了也能跟上。
    upstream: String,
    /// 被名单拒绝的连接数。
    rejected: AtomicU64,
}

impl FederationProtocol {
    /// `upstream` 是本机 Tuwunel 的联邦地址（主机名:端口）。
    pub fn new(allowlist: Allowlist, upstream: String) -> Self {
        Self {
            allowlist,
            upstream,
            rejected: AtomicU64::new(0),
        }
    }

    /// 被名单拒绝的连接数：连接来自 endpoint ID 不在服务器名单里的对端。
    pub fn rejected(&self) -> u64 {
        self.rejected.load(Ordering::Relaxed)
    }

    /// 接受这条连接上的流，直到连接关闭或对方从名单里被去掉。返回结束原因。
    async fn serve(
        &self,
        conn: &Connection,
        remote: EndpointId,
        mut allowlist: watch::Receiver<HashSet<EndpointId>>,
    ) -> &'static str {
        let mut streams = JoinSet::new();
        loop {
            tokio::select! {
                accepted = conn.accept_bi() => match accepted {
                    Ok((send, recv)) => {
                        streams.spawn(serve_stream(send, recv, self.upstream.clone()));
                    }
                    Err(err) => return connection_log_code(&err),
                },
                changed = allowlist.changed() => {
                    if changed.is_err() || !allowlist.borrow_and_update().contains(&remote) {
                        conn.close(CLOSE_UNAUTHORIZED, REASON_UNAUTHORIZED);
                        return "the peer is no longer in the server list";
                    }
                }
                // 及时回收结束的流任务，长连接上的 JoinSet 才不会越积越多。
                Some(joined) = streams.join_next() => {
                    if let Err(err) = joined
                        && err.is_panic()
                    {
                        error!(remote = %remote.fmt_short(), "federation stream task panicked: {err}");
                    }
                }
            }
        }
    }
}

impl ProtocolHandler for FederationProtocol {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        let remote = conn.remote_id();
        // 先订阅再检查：检查之后、订阅之前发生的删除也会被看到。
        let mut allowlist = self.allowlist.subscribe();
        if !allowlist.borrow_and_update().contains(&remote) {
            self.rejected.fetch_add(1, Ordering::Relaxed);
            warn!(remote = %remote, "rejected a connection not in the server list");
            conn.close(CLOSE_UNAUTHORIZED, REASON_UNAUTHORIZED);
            return Ok(());
        }
        conn.set_max_concurrent_bi_streams(MAX_CONCURRENT_STREAMS);
        info!(remote = %remote.fmt_short(), "peer server connected");
        let reason = self.serve(&conn, remote, allowlist).await;
        info!(remote = %remote.fmt_short(), %reason, "peer server connection ended");
        Ok(())
    }
}

async fn serve_stream(mut send: SendStream, mut recv: RecvStream, upstream: String) {
    let mut kind = [0u8; 1];
    if recv.read_exact(&mut kind).await.is_err() {
        debug!("the stream ended before the request type");
        return;
    }
    match Request::from_byte(kind[0]) {
        Some(Request::Forward) => forward_to_upstream(send, recv, &upstream).await,
        Some(Request::Ping) => reply(&mut send, Status::Ok).await,
        Some(Request::Port | Request::Udp) | None => {
            warn!(kind = kind[0], "unsupported request type");
            reply(&mut send, Status::UnsupportedRequest).await;
        }
    }
}

async fn forward_to_upstream(mut send: SendStream, mut recv: RecvStream, upstream: &str) {
    // 入口容器运行时会被对账删掉重建（clients 上的地址是动态分配的）：每条入站流现解析一次，
    // 重建后新流自动跟上，不用重启核心。
    let resolved: Vec<std::net::SocketAddr> = match tokio::net::lookup_host(upstream).await {
        Ok(addrs) => addrs.collect(),
        Err(err) => {
            warn!(%upstream, "cannot resolve the local Tuwunel federation address: {err}");
            reply(&mut send, Status::UpstreamUnavailable).await;
            return;
        }
    };
    let Some(addr) = resolved
        .iter()
        .find(|addr| addr.is_ipv4())
        .or(resolved.first())
        .copied()
    else {
        warn!(%upstream, "the local Tuwunel federation address resolved to nothing");
        reply(&mut send, Status::UpstreamUnavailable).await;
        return;
    };
    let mut tcp = match TcpStream::connect(addr).await {
        Ok(tcp) => tcp,
        Err(err) => {
            warn!(%upstream, %addr, "cannot reach the local Tuwunel federation port: {err}");
            reply(&mut send, Status::UpstreamUnavailable).await;
            return;
        }
    };
    if let Err(err) = tcp.set_nodelay(true) {
        debug!("failed to set TCP_NODELAY on the upstream connection: {err}");
    }
    if send.write_all(&[Status::Ok as u8]).await.is_err() {
        debug!("failed to write the status");
        forward::abort(&mut send, &mut recv);
        return;
    }
    let (mut tcp_read, mut tcp_write) = tcp.split();
    let result = tokio::try_join!(
        forward::tunnel_to_tcp(&mut recv, &mut tcp_write, None),
        forward::tcp_to_tunnel(&mut tcp_read, &mut send),
    );
    if let Err(err) = result {
        debug!(error = %err.log_safe(), "forwarding interrupted");
        forward::abort(&mut send, &mut recv);
    }
}

async fn reply(send: &mut SendStream, status: Status) {
    match send.write_all(&[status as u8]).await {
        Ok(()) => {
            send.finish().ok();
        }
        Err(_) => debug!("failed to write the status"),
    }
}
