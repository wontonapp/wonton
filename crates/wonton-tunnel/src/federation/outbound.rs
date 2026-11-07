//! 联邦出站：在 Tuwunel 的网络上监听，按 TLS ClientHello 的 SNI 把连接转给配对过的对方服务器。
//!
//! 每条 TCP 连接先读到 ClientHello 为止（rustls 的 [`Acceptor`]，只解析、不回握手、不终结 TLS），SNI
//! 就是对方的 Matrix 服务器名；按嵌入方给的查找函数换成配对时记下的 endpoint ID，经共享的 iroh 连接
//! 转发。转发器不看 ClientHello 之后的内容，断开也不当协议错误。

use std::{
    collections::BTreeSet,
    io,
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use iroh::{Endpoint, EndpointAddr, EndpointId};
use rustls::server::Acceptor;
use tokio::{
    io::AsyncReadExt,
    net::{TcpListener, TcpStream},
    sync::watch,
    task::AbortHandle,
};
use tracing::{debug, info, warn};

use crate::{forward, wire::Status};

use super::peer::{ForwardStream, PeerStates, PeerTable};

/// 嵌入方提供的查找函数：Matrix 服务器名 → 配对时记下的对方 endpoint ID 和地址。
///
/// 返回 `None` 表示这个名字不在配对名单里（还没配对，或者对方已被删除），连接会被关闭并记一次
/// [`OutboundErrors::unknown_server`]。绑定只在配对时定下、以后不变（ADR-0050），嵌入方按
/// `core/peers.json` 实现，不做别的解析（不查 DNS）。
pub type ServerLookup = Arc<dyn Fn(&str) -> Option<(EndpointId, EndpointAddr)> + Send + Sync>;

/// 出站一侧记下来的错误次数。这些错误没有对应的对方服务器，进不了每台服务器的状态，只能计数。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OutboundErrors {
    /// SNI 指向的服务器名不在配对名单里，连接已关闭。
    pub unknown_server: u64,
    /// 连接开头不是能读出 SNI 的 TLS ClientHello（不是 TLS、格式不对、没有 SNI、太大或超时），
    /// 连接已关闭。
    pub invalid_client_hello: u64,
}

#[derive(Debug, Default)]
struct Counters {
    unknown_server: AtomicU64,
    invalid_client_hello: AtomicU64,
}

/// 联邦出站转发器：在 `listen` 上接受本机 Tuwunel 发往对方服务器的连接，按 SNI 经 iroh 转出去；
/// 配对名单里每出现一台服务器就建它的客户端并开始探活，不等出站流量。
#[derive(Debug)]
pub struct FederationForwarder {
    peers: Arc<PeerTable>,
    status: watch::Sender<PeerStates>,
    counters: Arc<Counters>,
    listener: AbortHandle,
    sync: AbortHandle,
    listen_addr: SocketAddr,
}

/// 转发任务共享的参数。
pub(super) struct ForwarderCtx {
    pub(super) endpoint: Endpoint,
    pub(super) lookup: ServerLookup,
    pub(super) peers: Arc<PeerTable>,
    pub(super) status: watch::Sender<PeerStates>,
    pub(super) ping_interval: Duration,
    pub(super) ping_timeout: Duration,
}

impl FederationForwarder {
    /// 在 `listen` 上开始转发。`paired` 是已配对的服务器名：名单里出现的每台马上建客户端并探活，
    /// 名单里消失的拆掉。`ping_interval` 和 `ping_timeout` 是对每台对方服务器的定时探活周期和每次
    /// 探活的超时。
    pub async fn bind(
        endpoint: Endpoint,
        listen: SocketAddr,
        lookup: ServerLookup,
        paired: watch::Receiver<BTreeSet<String>>,
        ping_interval: Duration,
        ping_timeout: Duration,
    ) -> io::Result<Self> {
        let listener = TcpListener::bind(listen).await?;
        let listen_addr = listener.local_addr()?;
        let (status, _) = watch::channel(PeerStates::new());
        let counters = Arc::new(Counters::default());
        let ctx = Arc::new(ForwarderCtx {
            endpoint,
            lookup,
            peers: Arc::new(PeerTable::default()),
            status: status.clone(),
            ping_interval,
            ping_timeout,
        });
        let task = tokio::spawn(accept_loop(listener, ctx.clone(), counters.clone()));
        let sync = tokio::spawn(sync_loop(paired, ctx.clone())).abort_handle();
        Ok(Self {
            peers: ctx.peers.clone(),
            status,
            counters,
            listener: task.abort_handle(),
            sync,
            listen_addr,
        })
    }

    /// 转发器监听的地址。
    pub fn listen_addr(&self) -> SocketAddr {
        self.listen_addr
    }

    /// 订阅各台对方服务器的状态。
    pub fn watch_status(&self) -> watch::Receiver<PeerStates> {
        self.status.subscribe()
    }

    /// 出站一侧记下的错误次数。
    pub fn outbound_errors(&self) -> OutboundErrors {
        OutboundErrors {
            unknown_server: self.counters.unknown_server.load(Ordering::Relaxed),
            invalid_client_hello: self.counters.invalid_client_hello.load(Ordering::Relaxed),
        }
    }
}

impl Drop for FederationForwarder {
    fn drop(&mut self) {
        self.listener.abort();
        self.sync.abort();
        self.peers.stop_all();
    }
}

/// 配对名单变了就跟着变：新出现的服务器马上建客户端、立刻开始探活（不等出站流量，否则没有联邦流量的
/// 时候状态一直空着）；从名单里删掉的、查询已经查不到的马上拆掉。
async fn sync_loop(mut paired: watch::Receiver<BTreeSet<String>>, ctx: Arc<ForwarderCtx>) {
    loop {
        let names = paired.borrow_and_update().clone();
        ctx.peers.retain(&names);
        for name in &names {
            let Some((id, addr)) = (ctx.lookup)(name) else {
                continue;
            };
            let peer = ctx.peers.get_or_create(name, id, addr.clone(), &ctx);
            peer.set_target(id, addr);
        }
        if paired.changed().await.is_err() {
            return;
        }
    }
}

async fn accept_loop(listener: TcpListener, ctx: Arc<ForwarderCtx>, counters: Arc<Counters>) {
    loop {
        match listener.accept().await {
            Ok((tcp, _)) => {
                let (ctx, counters) = (ctx.clone(), counters.clone());
                tokio::spawn(serve_conn(tcp, ctx, counters));
            }
            Err(err) => {
                warn!("federation outbound listener failed: {err}");
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }
}

async fn serve_conn(mut tcp: TcpStream, ctx: Arc<ForwarderCtx>, counters: Arc<Counters>) {
    let _ = tcp.set_nodelay(true);
    let (hello, name) = match read_client_hello(&mut tcp).await {
        Ok(result) => result,
        Err(reason) => {
            counters
                .invalid_client_hello
                .fetch_add(1, Ordering::Relaxed);
            warn!(%reason, "federation outbound: connection does not start with a usable TLS ClientHello");
            return;
        }
    };
    let Some((id, addr)) = (ctx.lookup)(&name) else {
        counters.unknown_server.fetch_add(1, Ordering::Relaxed);
        warn!(%name, "federation outbound: SNI not in the paired server list, closing the connection");
        return;
    };
    let peer = ctx.peers.get_or_create(&name, id, addr.clone(), &ctx);
    peer.set_target(id, addr);
    let mut stream = match peer.open_forward().await {
        Ok(stream) => stream,
        Err(err) => {
            debug!(server = %name, "federation outbound: cannot reach the peer: {err}");
            return;
        }
    };
    // ClientHello 之前的字节原样转过去（同一段 TCP 里 ClientHello 之后紧跟的字节也在里面）。
    if let Err(err) = stream.send.write_all(&hello).await {
        debug!(server = %name, "federation outbound: failed to send the ClientHello: {err}");
        stream.abort();
        return;
    }
    // 对方先回 1 字节状态，然后是它那台 Tuwunel 的原样字节（TLS 在两台 Tuwunel 之间）。
    let mut status = [0u8; 1];
    match stream.recv.read_exact(&mut status).await {
        Err(err) => {
            debug!(server = %name, "federation outbound: failed to read the status: {err}");
            stream.abort();
        }
        Ok(_) => match Status::from_byte(status[0]) {
            Some(Status::Ok) => {
                peer.note_confirmed(stream.conn_id);
                relay(stream, tcp).await;
            }
            Some(Status::UpstreamUnavailable) => {
                info!(server = %name, "federation outbound: the peer cannot reach its Tuwunel");
                stream.abort();
            }
            _ => {
                warn!(server = %name, kind = status[0], "federation outbound: the peer does not support this request");
                stream.abort();
            }
        },
    }
}

/// ClientHello 已发完：之后的字节双向原样转发。
async fn relay(mut stream: ForwardStream, tcp: TcpStream) {
    let (mut tcp_read, mut tcp_write) = tcp.into_split();
    if let Err(err) = tokio::try_join!(
        forward::tcp_to_tunnel(&mut tcp_read, &mut stream.send),
        forward::tunnel_to_tcp(&mut stream.recv, &mut tcp_write),
    ) {
        debug!("federation forwarding interrupted: {err}");
        stream.abort();
    }
}

/// 读 ClientHello 的最长时间。连接来自本机 Tuwunel，正常毫秒级就到。
const HELLO_TIMEOUT: Duration = Duration::from_secs(10);

/// 缓冲的 TLS 字节上限。rustls 自己也把握手消息限制在 64 KiB。
const MAX_HELLO: usize = 64 * 1024 + 5;

/// 每次从连接里读多少字节。
const READ_CHUNK: usize = 4096;

/// 连接开头读不出 SNI 的原因。
#[derive(Debug, thiserror::Error)]
enum HelloError {
    #[error("the connection closed before the ClientHello: {0}")]
    Closed(io::Error),
    #[error("not TLS or an invalid ClientHello: {0}")]
    NotTls(io::Error),
    #[error("the ClientHello carries no SNI")]
    NoSni,
    #[error("the ClientHello exceeds the size limit")]
    TooLarge,
    #[error("the ClientHello did not complete within the time limit")]
    Timeout,
}

/// 读到 TLS ClientHello 为止，返回（已读的全部字节，SNI）。已读的字节要原样转给对方，之后的字节由
/// 调用方从连接里继续读。不回握手：拿到 SNI 就由调用方决定转给谁，TLS 由两台 Tuwunel 端到端完成。
async fn read_client_hello(tcp: &mut TcpStream) -> Result<(Vec<u8>, String), HelloError> {
    let mut acceptor = Acceptor::default();
    let mut buf = Vec::new();
    let mut consumed = 0;
    let deadline = tokio::time::Instant::now() + HELLO_TIMEOUT;
    loop {
        // 先喂已经读到的字节：连接开头可能一次带来不止一条 TLS 记录。
        while consumed < buf.len() {
            let mut view: &[u8] = &buf[consumed..];
            // read_tls 每次最多吃进一条记录，返回吃掉的字节数；剩下的留在 view 里下轮再喂。
            let used = acceptor.read_tls(&mut view).map_err(HelloError::NotTls)?;
            if used == 0 {
                return Err(HelloError::NotTls(io::Error::other(
                    "could not read a complete record",
                )));
            }
            consumed += used;
            match acceptor.accept() {
                Ok(Some(accepted)) => {
                    let hello = accepted.client_hello();
                    let Some(name) = hello.server_name() else {
                        return Err(HelloError::NoSni);
                    };
                    return Ok((buf, name.to_owned()));
                }
                Ok(None) => {}
                Err((err, _alert)) => {
                    // 对方不是 TLS 客户端或握手不合法。告警由调用方记，不回 TLS 告警。
                    return Err(HelloError::NotTls(io::Error::other(err)));
                }
            }
        }
        // 已缓冲的字节喂完了：从连接里再读一段。
        let mut chunk = [0u8; READ_CHUNK];
        let n = tokio::time::timeout_at(deadline, tcp.read(&mut chunk))
            .await
            .map_err(|_| HelloError::Timeout)?
            .map_err(HelloError::Closed)?;
        if n == 0 {
            return Err(HelloError::Closed(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "the connection closed before the ClientHello",
            )));
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.len() > MAX_HELLO {
            return Err(HelloError::TooLarge);
        }
    }
}
