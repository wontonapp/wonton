//! 隧道服务端：只接受白名单里的设备，把每条流转发到本机的上游 HTTP 服务，或者连同对端身份交给嵌入方。

use std::{
    collections::HashSet,
    net::SocketAddr,
    sync::{Arc, atomic::AtomicU64},
    time::Duration,
};

use iroh::{
    Endpoint, EndpointId,
    endpoint::{Connection, RecvStream, SendStream, VarInt},
    protocol::{AcceptError, DynProtocolHandler, ProtocolHandler, Router},
};
use tokio::{
    net::TcpStream,
    sync::{mpsc, watch},
    task::JoinSet,
    time::timeout,
};
use tracing::{debug, error, info, warn};

use crate::{
    allowlist::Allowlist,
    embed::{PeerStream, TunnelListener},
    error::connection_log_code,
    forward,
    pair::{PAIR_ALPN, PairHandler, PairProtocol},
    ports::{PortResolver, UnsupportedPortResolver},
    udp::server::Flows,
    wire::{
        ALPN, CLOSE_UNAUTHORIZED, MAX_PORT_REQUEST_LEN, PortRequest, REASON_UNAUTHORIZED, Request,
        Status,
    },
};

/// 每条连接上客户端可以同时打开的流数。本地代理的每条 TCP 连接占一条流，HTTP keep-alive、长轮询和
/// WebSocket 都会长期占用，所以比 QUIC 默认的 100 放宽。
///
/// 结束的流腾出的额度，noq 1.4 要攒到超过 1/8（这里是 33 条）才发 MAX_STREAMS 告诉客户端
/// （`StreamsState::queue_max_stream_id`）：同时开着 224 条以上时，新流要等到再有流结束才开得出来。
const MAX_CONCURRENT_STREAMS: VarInt = VarInt::from_u32(256);

/// [`TunnelServer::shutdown`] 等 endpoint 真正关掉的上限。正常是毫秒级；等不到也不挂住调用方，
/// 只是返回时端口可能还被占着。
const CLOSE_WAIT: Duration = Duration::from_secs(10);

/// 隧道流的去向。
#[derive(Debug, Clone)]
enum Upstream {
    /// 转发到本机的 TCP 服务。
    Tcp(SocketAddr),
    /// 交给嵌入方，由它从 [`TunnelListener`] 取走。
    Embedded(mpsc::Sender<PeerStream>),
}

/// 隧道服务端协议，挂在 iroh [`Router`] 的 [`ALPN`] 上。
///
/// 独立运行时用 [`TunnelServer`]；核心会把它和配对等其他协议挂在同一个 endpoint 的 router 上。
#[derive(Debug)]
pub struct TunnelProtocol<R: PortResolver = UnsupportedPortResolver> {
    allowlist: Allowlist,
    upstream: Upstream,
    resolver: Arc<R>,
    timeouts: crate::udp::server::FlowTimeouts,
    oversized: Arc<AtomicU64>,
}

impl<R: PortResolver> Clone for TunnelProtocol<R> {
    fn clone(&self) -> Self {
        Self {
            allowlist: self.allowlist.clone(),
            upstream: self.upstream.clone(),
            resolver: self.resolver.clone(),
            timeouts: self.timeouts,
            oversized: self.oversized.clone(),
        }
    }
}

impl TunnelProtocol {
    /// `upstream` 是隧道流量要转发到的本机 HTTP 服务地址。
    pub fn new(allowlist: Allowlist, upstream: SocketAddr) -> Self {
        Self::with_resolver(allowlist, upstream, Arc::new(UnsupportedPortResolver))
    }

    /// 插件端口由嵌入方解析。
    pub fn with_resolver<R: PortResolver>(
        allowlist: Allowlist,
        upstream: SocketAddr,
        resolver: Arc<R>,
    ) -> TunnelProtocol<R> {
        TunnelProtocol {
            allowlist,
            upstream: Upstream::Tcp(upstream),
            resolver,
            timeouts: crate::udp::server::FlowTimeouts::default(),
            oversized: Arc::new(AtomicU64::new(0)),
        }
    }

    /// 不转发到 TCP 上游，而是把每条流连同对端设备的 endpoint ID 交给嵌入方，从返回的
    /// [`TunnelListener`] 取。核心用它在进程内服务网关，并知道请求来自哪台已配对设备。
    pub fn embedded(allowlist: Allowlist) -> (Self, TunnelListener) {
        Self::embedded_with(allowlist, Arc::new(UnsupportedPortResolver))
    }

    /// 同 [`Self::embedded`]，插件端口由嵌入方解析。
    pub fn embedded_with<R: PortResolver>(
        allowlist: Allowlist,
        resolver: Arc<R>,
    ) -> (TunnelProtocol<R>, TunnelListener) {
        let (streams, listener) = TunnelListener::channel();
        let protocol = TunnelProtocol {
            allowlist,
            upstream: Upstream::Embedded(streams),
            resolver,
            timeouts: crate::udp::server::FlowTimeouts::default(),
            oversized: Arc::new(AtomicU64::new(0)),
        };
        (protocol, listener)
    }
}

impl<R: PortResolver> TunnelProtocol<R> {
    /// 换掉 UDP 空闲和分片等待时限，测试用短值。
    pub fn with_udp_timeouts(mut self, timeouts: crate::udp::server::FlowTimeouts) -> Self {
        self.timeouts = timeouts;
        self
    }

    /// 超过两片数据报预算的 UDP 包数。
    pub fn oversized_drops(&self) -> u64 {
        self.oversized.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// 接受这条连接上的流，直到连接关闭或设备被吊销。返回结束原因。
    async fn serve(
        &self,
        conn: &Connection,
        remote: EndpointId,
        mut allowlist: watch::Receiver<HashSet<EndpointId>>,
    ) -> &'static str {
        let mut streams = JoinSet::new();
        let flows = Flows::start(conn.clone(), self.timeouts, self.oversized.clone());
        loop {
            tokio::select! {
                accepted = conn.accept_bi() => match accepted {
                    Ok((send, recv)) => {
                        streams.spawn(serve_stream(
                            send,
                            recv,
                            remote,
                            flows.clone(),
                            self.upstream.clone(),
                            self.resolver.clone(),
                        ));
                    }
                    Err(err) => return connection_log_code(&err),
                },
                changed = allowlist.changed() => {
                    if changed.is_err() || !allowlist.borrow_and_update().contains(&remote) {
                        conn.close(CLOSE_UNAUTHORIZED, REASON_UNAUTHORIZED);
                        return "device revoked";
                    }
                }
                // 及时回收结束的流任务，长连接上的 JoinSet 才不会越积越多。
                Some(joined) = streams.join_next() => {
                    if let Err(err) = joined
                        && err.is_panic()
                    {
                        error!(remote = %remote.fmt_short(), "tunnel stream task panicked: {err}");
                    }
                }
            }
        }
    }
}

impl<R: PortResolver> ProtocolHandler for TunnelProtocol<R> {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        let remote = conn.remote_id();
        // 先订阅再检查：检查之后、订阅之前发生的吊销也会被看到。
        let mut allowlist = self.allowlist.subscribe();
        if !allowlist.borrow_and_update().contains(&remote) {
            warn!(remote = %remote, "rejected an unpaired device");
            conn.close(CLOSE_UNAUTHORIZED, REASON_UNAUTHORIZED);
            return Ok(());
        }
        conn.set_max_concurrent_bi_streams(MAX_CONCURRENT_STREAMS);
        info!(remote = %remote.fmt_short(), "device connected");
        let reason = self.serve(&conn, remote, allowlist).await;
        info!(remote = %remote.fmt_short(), %reason, "device connection ended");
        Ok(())
    }
}

async fn serve_stream<R: PortResolver>(
    mut send: SendStream,
    mut recv: RecvStream,
    remote: EndpointId,
    flows: Arc<Flows>,
    upstream: Upstream,
    resolver: Arc<R>,
) {
    let mut kind = [0u8; 1];
    if recv.read_exact(&mut kind).await.is_err() {
        debug!("the stream ended before the request type");
        return;
    }
    match Request::from_byte(kind[0]) {
        Some(Request::Forward) => match upstream {
            Upstream::Tcp(addr) => forward_to_upstream(send, recv, addr).await,
            Upstream::Embedded(streams) => hand_over(send, recv, remote, &streams).await,
        },
        Some(Request::Ping) => reply(&mut send, Status::Ok).await,
        Some(kind @ (Request::Port | Request::Udp)) => {
            serve_port(send, recv, kind, flows, resolver.as_ref()).await;
        }
        None => {
            warn!(kind = kind[0], "unsupported request type");
            reply(&mut send, Status::UnsupportedRequest).await;
        }
    }
}

async fn read_port_request<'a>(
    recv: &mut RecvStream,
    kind: Request,
    header: &'a mut [u8; MAX_PORT_REQUEST_LEN],
) -> Result<PortRequest<'a>, String> {
    header[0] = kind as u8;
    recv.read_exact(&mut header[1..2])
        .await
        .map_err(|err| err.to_string())?;
    let name_len = usize::from(header[1]);
    if name_len > crate::wire::MAX_PLUGIN_ID_LEN {
        return Err("plugin identifier exceeds 63 bytes".to_owned());
    }
    let len = name_len + 4;
    recv.read_exact(&mut header[2..len])
        .await
        .map_err(|err| err.to_string())?;
    PortRequest::decode(&header[..len])
        .map(|(request, _)| request)
        .map_err(|err| err.to_string())
}

async fn serve_port<R: PortResolver>(
    mut send: SendStream,
    mut recv: RecvStream,
    kind: Request,
    flows: Arc<Flows>,
    resolver: &R,
) {
    let mut header = [0; MAX_PORT_REQUEST_LEN];
    let request = match read_port_request(&mut recv, kind, &mut header).await {
        Ok(request) => request,
        Err(err) => {
            debug!("invalid plugin port request: {err}");
            reply(&mut send, Status::UnsupportedRequest).await;
            return;
        }
    };
    let protocol = match kind {
        Request::Port => wonton_protocol::PortProtocol::Tcp,
        _ => wonton_protocol::PortProtocol::Udp,
    };
    let addr = match resolver
        .resolve(request.plugin, request.port, protocol)
        .await
    {
        Ok(addr) => addr,
        Err(err) => {
            reply(&mut send, err.status()).await;
            return;
        }
    };
    match kind {
        Request::Port => connect_and_forward(send, recv, addr).await,
        _ => {
            flows
                .serve(send, recv, addr, (request.plugin, request.port))
                .await
        }
    }
}

async fn forward_to_upstream(send: SendStream, recv: RecvStream, upstream: SocketAddr) {
    connect_and_forward(send, recv, upstream).await;
}

async fn connect_and_forward(mut send: SendStream, mut recv: RecvStream, upstream: SocketAddr) {
    let mut tcp = match TcpStream::connect(upstream).await {
        Ok(tcp) => tcp,
        Err(err) => {
            warn!(%upstream, "failed to reach the upstream HTTP service: {err}");
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

/// 把流交给嵌入方。先在队列里占到位置再回“成功”：嵌入方已经停止时，如实回“上游不可用”。
async fn hand_over(
    mut send: SendStream,
    mut recv: RecvStream,
    remote: EndpointId,
    streams: &mpsc::Sender<PeerStream>,
) {
    let Ok(slot) = streams.reserve().await else {
        warn!("the embedder has stopped accepting tunnel streams");
        reply(&mut send, Status::UpstreamUnavailable).await;
        return;
    };
    if send.write_all(&[Status::Ok as u8]).await.is_err() {
        debug!("failed to write the status");
        forward::abort(&mut send, &mut recv);
        return;
    }
    slot.send(PeerStream::new(remote, send, recv));
}

pub(crate) async fn reply(send: &mut SendStream, status: Status) {
    match send.write_all(&[status as u8]).await {
        Ok(()) => {
            send.finish().ok();
        }
        Err(_) => debug!("failed to write the status"),
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
        Self::with_protocol(endpoint, allowlist, protocol)
    }

    /// 同 [`spawn`](Self::spawn)，插件端口按 `resolver` 解析。
    pub fn spawn_with_resolver<R: PortResolver>(
        endpoint: Endpoint,
        allowlist: Allowlist,
        upstream: SocketAddr,
        resolver: Arc<R>,
    ) -> Self {
        let protocol = TunnelProtocol::with_resolver(allowlist.clone(), upstream, resolver);
        Self::with_protocol(endpoint, allowlist, protocol)
    }

    /// 在 `endpoint` 上开始接受隧道连接，只放行 `allowlist` 里的设备，每条流连同对端身份交给嵌入方，
    /// 从返回的 [`TunnelListener`] 取。
    pub fn spawn_embedded(endpoint: Endpoint, allowlist: Allowlist) -> (Self, TunnelListener) {
        let (protocol, listener) = TunnelProtocol::embedded(allowlist.clone());
        (Self::with_protocol(endpoint, allowlist, protocol), listener)
    }

    /// 同 [`spawn_embedded`](Self::spawn_embedded)，同一个 endpoint 上再接受还没登记的设备来配对：配对成功的
    /// 设备由 `pairing` 的处理方加进 `allowlist`，随后用隧道的 ALPN 连进来。
    pub fn spawn_embedded_with_pairing<H: PairHandler>(
        endpoint: Endpoint,
        allowlist: Allowlist,
        pairing: PairProtocol<H>,
    ) -> (Self, TunnelListener) {
        let (protocol, listener) = TunnelProtocol::embedded(allowlist.clone());
        let router = Router::builder(endpoint)
            .accept(ALPN, protocol)
            .accept(PAIR_ALPN, pairing)
            .spawn();
        (Self { router, allowlist }, listener)
    }

    /// 同 [`spawn_embedded_with_pairing`](Self::spawn_embedded_with_pairing)，同一个 endpoint 上再多挂几个
    /// 协议的处理器（如联邦转发和服务器配对），每项是它的 ALPN 加处理器。
    pub fn spawn_embedded_with_pairing_and<H: PairHandler>(
        endpoint: Endpoint,
        allowlist: Allowlist,
        pairing: PairProtocol<H>,
        extra: impl IntoIterator<Item = (&'static [u8], Box<dyn DynProtocolHandler>)>,
    ) -> (Self, TunnelListener) {
        Self::spawn_embedded_with_pairing_and_resolver(
            endpoint,
            allowlist,
            pairing,
            extra,
            Arc::new(UnsupportedPortResolver),
        )
    }

    /// 同 [`spawn_embedded_with_pairing_and`](Self::spawn_embedded_with_pairing_and)，插件端口按 `resolver` 解析。
    pub fn spawn_embedded_with_pairing_and_resolver<H: PairHandler, R: PortResolver>(
        endpoint: Endpoint,
        allowlist: Allowlist,
        pairing: PairProtocol<H>,
        extra: impl IntoIterator<Item = (&'static [u8], Box<dyn DynProtocolHandler>)>,
        resolver: Arc<R>,
    ) -> (Self, TunnelListener) {
        let (protocol, listener) = TunnelProtocol::embedded_with(allowlist.clone(), resolver);
        let mut builder = Router::builder(endpoint)
            .accept(ALPN, protocol)
            .accept(PAIR_ALPN, pairing);
        for (alpn, handler) in extra {
            builder = builder.accept(alpn, handler);
        }
        (
            Self {
                router: builder.spawn(),
                allowlist,
            },
            listener,
        )
    }

    fn with_protocol<R: PortResolver>(
        endpoint: Endpoint,
        allowlist: Allowlist,
        protocol: TunnelProtocol<R>,
    ) -> Self {
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
    ///
    /// 返回时 UDP 端口已经空出来：iroh 的 `Endpoint::close` 在有别的关闭在进行时会先返回，套接字要等那次
    /// 关闭走完才释放，所以这里再等 endpoint 真的关掉（有上限，不把调用方挂住）。
    pub async fn shutdown(self) {
        let closed = self.router.endpoint().closed();
        match self.router.shutdown().await {
            Ok(()) => {}
            Err(err) if err.is_panic() => std::panic::resume_unwind(err.into_panic()),
            Err(err) => error!("tunnel server shutdown failed: {err}"),
        }
        if timeout(CLOSE_WAIT, closed).await.is_err() {
            warn!("endpoint 没有在上限内关掉，UDP 端口可能还被占着");
        }
    }
}
