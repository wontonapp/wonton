//! 联邦入站：只接受服务器名单里的对方服务器，把每条流原样接到本机 Tuwunel 的联邦端口。
//!
//! 线协议和隧道的一样（见 crate 级文档的“线协议”），但白名单是服务器名单：配对过的服务器进不了
//! App 的隧道，配对过的设备也进不了联邦（ADR-0050 的“名单分开”）。

use std::{
    collections::HashSet,
    net::SocketAddr,
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
    /// 本机 Tuwunel 的联邦端口（8448），入站的流都接到它。
    upstream: SocketAddr,
    /// 被名单拒绝的连接数。
    rejected: AtomicU64,
}

impl FederationProtocol {
    /// `upstream` 是本机 Tuwunel 的联邦端口。
    pub fn new(allowlist: Allowlist, upstream: SocketAddr) -> Self {
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
                        return "对方已不在服务器名单里".to_owned();
                    }
                }
                // 及时回收结束的流任务，长连接上的 JoinSet 才不会越积越多。
                Some(joined) = streams.join_next() => {
                    if let Err(err) = joined
                        && err.is_panic()
                    {
                        error!(remote = %remote.fmt_short(), "联邦流任务 panic：{err}");
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
            warn!(remote = %remote, "拒绝不在服务器名单里的连接");
            conn.close(CLOSE_UNAUTHORIZED, REASON_UNAUTHORIZED);
            return Ok(());
        }
        conn.set_max_concurrent_bi_streams(MAX_CONCURRENT_STREAMS);
        info!(remote = %remote.fmt_short(), "对方服务器已连接");
        let reason = self.serve(&conn, remote, allowlist).await;
        info!(remote = %remote.fmt_short(), %reason, "对方服务器连接结束");
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
            warn!(%upstream, "连不上本机 Tuwunel 的联邦端口：{err}");
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
