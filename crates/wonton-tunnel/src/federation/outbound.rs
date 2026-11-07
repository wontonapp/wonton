//! 联邦出站：在核心的内部网络上开一个 HTTP CONNECT 代理，本机 Tuwunel 的联邦出站都经它走（Tuwunel 的
//! `proxy` 配置），按 CONNECT 里的服务器名转给配对过的对方服务器。
//!
//! 每条 TCP 连接先读 CONNECT 请求头，目标的主机名就是对方的 Matrix 服务器名；按嵌入方给的查找函数换成
//! 配对时记下的 endpoint ID，经共享的 iroh 连接转发。对方确认接上它那台 Tuwunel 之后才回 200，之后的
//! 字节（两台 Tuwunel 之间的 TLS）双向原样转发，转发器不终结 TLS，断开也不当协议错误。
//!
//! 只接 8448：Tuwunel 先 CONNECT `<名字>:443` 取 well-known，这里回 502，它就改连 Matrix 的默认联邦端口
//! `<名字>:8448`（实测见 ADR-0050 的“验证”）。转发器不查 DNS，名字在配对名单里才转。
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
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::watch,
    task::AbortHandle,
};
use tracing::{debug, info, warn};

use crate::{forward, wire::Status};

use super::peer::{ForwardStream, PeerStates, PeerTable};

/// 嵌入方提供的查找函数：Matrix 服务器名 → 配对时记下的对方 endpoint ID 和地址。
///
/// 返回 `None` 表示这个名字不在配对名单里（还没配对，或者对方已被删除），连接回 403 后关闭，并记一次
/// [`OutboundErrors::unknown_server`]。绑定只在配对时定下、以后不变（ADR-0050），嵌入方按
/// `core/peers.json` 实现，不做别的解析（不查 DNS）。
pub type ServerLookup = Arc<dyn Fn(&str) -> Option<(EndpointId, EndpointAddr)> + Send + Sync>;

/// 出站一侧记下来的错误次数。这些错误没有对应的对方服务器，进不了每台服务器的状态，只能计数。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OutboundErrors {
    /// CONNECT 到联邦端口的服务器名不在配对名单里，回 403 后关闭。
    pub unknown_server: u64,
    /// 连接开头不是能读懂的 CONNECT 请求（格式不对、不是 CONNECT、目标不是 `主机:端口`、太大或超时），
    /// 回 400 后关闭。
    pub invalid_request: u64,
}

#[derive(Debug, Default)]
struct Counters {
    unknown_server: AtomicU64,
    invalid_request: AtomicU64,
}

/// 联邦出站转发器：在 `listen` 上接受本机 Tuwunel 发往对方服务器的 CONNECT，经 iroh 转出去；
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
            invalid_request: self.counters.invalid_request.load(Ordering::Relaxed),
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
    let request = match read_connect(&mut tcp).await {
        Ok(request) => request,
        Err(reason) => {
            counters.invalid_request.fetch_add(1, Ordering::Relaxed);
            warn!(%reason, "federation outbound: the connection does not start with a usable CONNECT request");
            respond(&mut tcp, BAD_REQUEST).await;
            return;
        }
    };
    let name = request.host;
    if request.port != FEDERATION_PORT {
        // well-known（443）这类请求：回 502，Tuwunel 就改连联邦端口。
        debug!(server = %name, port = request.port, "federation outbound: not the federation port");
        respond(&mut tcp, BAD_GATEWAY).await;
        return;
    }
    let Some((id, addr)) = (ctx.lookup)(&name) else {
        counters.unknown_server.fetch_add(1, Ordering::Relaxed);
        warn!(%name, "federation outbound: the server is not in the paired server list");
        respond(&mut tcp, FORBIDDEN).await;
        return;
    };
    let peer = ctx.peers.get_or_create(&name, id, addr.clone(), &ctx);
    peer.set_target(id, addr);
    let mut stream = match peer.open_forward().await {
        Ok(stream) => stream,
        Err(err) => {
            debug!(server = %name, "federation outbound: cannot reach the peer: {err}");
            respond(&mut tcp, BAD_GATEWAY).await;
            return;
        }
    };
    // 对方先回 1 字节状态，然后是它那台 Tuwunel 的原样字节（TLS 在两台 Tuwunel 之间）。
    let mut status = [0u8; 1];
    match stream.recv.read_exact(&mut status).await {
        Err(err) => {
            debug!(server = %name, "federation outbound: failed to read the status: {err}");
            stream.abort();
            respond(&mut tcp, BAD_GATEWAY).await;
        }
        Ok(_) => match Status::from_byte(status[0]) {
            Some(Status::Ok) => {
                peer.note_confirmed(stream.conn_id);
                established(stream, tcp, request.rest, &name).await;
            }
            Some(Status::UpstreamUnavailable) => {
                info!(server = %name, "federation outbound: the peer cannot reach its Tuwunel");
                stream.abort();
                respond(&mut tcp, BAD_GATEWAY).await;
            }
            _ => {
                warn!(server = %name, kind = status[0], "federation outbound: the peer does not support this request");
                stream.abort();
                respond(&mut tcp, BAD_GATEWAY).await;
            }
        },
    }
}

/// 对方已接上它的 Tuwunel：回 200，把请求头之后已经读到的字节先转过去，之后双向原样转发。
async fn established(mut stream: ForwardStream, mut tcp: TcpStream, rest: Vec<u8>, name: &str) {
    if let Err(err) = tcp.write_all(ESTABLISHED).await {
        debug!(server = %name, "federation outbound: failed to answer the CONNECT: {err}");
        stream.abort();
        return;
    }
    if !rest.is_empty()
        && let Err(err) = stream.send.write_all(&rest).await
    {
        debug!(server = %name, "federation outbound: failed to send the early bytes: {err}");
        stream.abort();
        return;
    }
    relay(stream, tcp).await;
}

/// CONNECT 已确认：之后的字节双向原样转发。
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

/// 回一个不带正文的错误状态，然后关闭（客户端可能已经走了，写失败不用管）。
async fn respond(tcp: &mut TcpStream, status: &str) {
    let reply = format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    let _ = tcp.write_all(reply.as_bytes()).await;
    let _ = tcp.shutdown().await;
}

const ESTABLISHED: &[u8] = b"HTTP/1.1 200 Connection established\r\n\r\n";
const BAD_REQUEST: &str = "400 Bad Request";
const FORBIDDEN: &str = "403 Forbidden";
const BAD_GATEWAY: &str = "502 Bad Gateway";

/// Matrix 的默认联邦端口：服务器名不带端口、没有 well-known 和 SRV 时，对方就在 `<名字>:8448`。
const FEDERATION_PORT: u16 = 8448;

/// 读请求头的最长时间。连接来自本机 Tuwunel，正常毫秒级就到。
const HEAD_TIMEOUT: Duration = Duration::from_secs(10);

/// 请求头的长度上限。CONNECT 只有一行请求行和几个头。
const MAX_HEAD: usize = 8 * 1024;

const MAX_HEADERS: usize = 32;

/// 读到的 CONNECT：目标和请求头之后已经读到的字节。
struct Connect {
    host: String,
    port: u16,
    rest: Vec<u8>,
}

/// 连接开头读不出 CONNECT 的原因。
#[derive(Debug, thiserror::Error)]
enum RequestError {
    #[error("the connection closed before the request head: {0}")]
    Closed(io::Error),
    #[error("not an HTTP request")]
    Malformed,
    #[error("the method is {0}, not CONNECT")]
    NotConnect(String),
    #[error("the CONNECT target is not host:port: {0}")]
    BadTarget(String),
    #[error("the request head exceeds the size limit")]
    TooLarge,
    #[error("the request head did not complete within the time limit")]
    Timeout,
}

/// 读到 CONNECT 请求头结束为止。
async fn read_connect(tcp: &mut TcpStream) -> Result<Connect, RequestError> {
    let deadline = tokio::time::Instant::now() + HEAD_TIMEOUT;
    let mut buf = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    loop {
        let n = tokio::time::timeout_at(deadline, tcp.read(&mut chunk))
            .await
            .map_err(|_| RequestError::Timeout)?
            .map_err(RequestError::Closed)?;
        if n == 0 {
            return Err(RequestError::Closed(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "the connection closed before the request head",
            )));
        }
        buf.extend_from_slice(&chunk[..n]);
        let mut headers = [httparse::EMPTY_HEADER; MAX_HEADERS];
        let mut request = httparse::Request::new(&mut headers);
        match request.parse(&buf) {
            Ok(httparse::Status::Complete(len)) => {
                let method = request.method.unwrap_or_default();
                if !method.eq_ignore_ascii_case("CONNECT") {
                    return Err(RequestError::NotConnect(method.to_owned()));
                }
                let target = request.path.unwrap_or_default();
                let (host, port) = target
                    .rsplit_once(':')
                    .and_then(|(host, port)| Some((host, port.parse::<u16>().ok()?)))
                    .filter(|(host, _)| !host.is_empty())
                    .ok_or_else(|| RequestError::BadTarget(target.to_owned()))?;
                return Ok(Connect {
                    host: host.to_ascii_lowercase(),
                    port,
                    rest: buf[len..].to_vec(),
                });
            }
            Ok(httparse::Status::Partial) if buf.len() < MAX_HEAD => {}
            Ok(httparse::Status::Partial) | Err(httparse::Error::TooManyHeaders) => {
                return Err(RequestError::TooLarge);
            }
            Err(_) => return Err(RequestError::Malformed),
        }
    }
}
