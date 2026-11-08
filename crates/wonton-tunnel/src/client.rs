//! 隧道客户端：所有请求复用一条到服务端的连接；连接断开后，下一次使用时自动重连。
//!
//! 同一时刻最多一次建连，失败后按 [`Pacing`] 退避，等待期间的请求共享下一次的结果（ADR-0032）。建连放在独立的
//! 任务里：等它的请求超时走了，建连也不会被丢下重来。

mod attempt;
mod suspend;

use attempt::Attempt;
pub use suspend::SuspendClock;

use std::{
    fmt,
    net::IpAddr,
    sync::{
        Arc, Mutex, MutexGuard, PoisonError, Weak,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant, SystemTime},
};

use iroh::{
    Endpoint, EndpointAddr, EndpointId, TransportAddr,
    endpoint::{Connection, ConnectionError, PathEventStream, RecvStream, SendStream},
};
use tokio::sync::watch;
use tracing::{debug, info};

use crate::{
    error::{
        TunnelError, from_connect_error, from_connection_error, from_read_exact_error,
        from_write_error,
    },
    forward,
    pacing::Pacing,
    wire::{ALPN, CLOSE_NORMAL, Request, Status},
};

/// 隧道客户端的连接状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TunnelStatus {
    /// 还没有连接过。第一次使用时自动连接。
    Idle,
    /// 正在建立连接，或者连接已建立但服务端还没确认本设备的授权。
    Connecting,
    /// 连接可用，服务端已确认授权。
    Connected,
    /// 连接失败或已断开，原因见错误；下一次使用时自动重连。
    Disconnected(TunnelError),
    /// 客户端已关闭，不再重连。
    Closed,
}

/// 传输路径的类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathKind {
    /// 局域网直连或打洞成功的 P2P 直连。
    Direct,
    /// 经中继服务器转发。
    Relay,
}

/// 当前连接正在使用的传输路径。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathInfo {
    /// 直连还是中继。
    pub kind: PathKind,
    /// 对端在这条路径上的地址：直连时是 IP 和端口，中继时是中继服务器。
    pub remote: TransportAddr,
    /// QUIC 对这条路径的往返时间估计。
    pub rtt: Duration,
}

impl PathInfo {
    /// 直连的对端是局域网地址。
    pub fn is_lan(&self) -> bool {
        let TransportAddr::Ip(addr) = &self.remote else {
            return false;
        };
        match addr.ip() {
            IpAddr::V4(ip) => ip.is_private() || ip.is_link_local() || ip.is_loopback(),
            IpAddr::V6(ip) => {
                ip.is_unique_local() || ip.is_unicast_link_local() || ip.is_loopback()
            }
        }
    }
}

/// 连接质量的指标，给连接页和日志用。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TunnelStats {
    /// 最近一次建连的耗时。
    pub handshake: Option<Duration>,
    /// 最后一次确认服务端有回应的时间。
    pub last_confirmed: Option<SystemTime>,
    /// 连接从可用变成不可用的次数。
    pub disconnects: u32,
    /// 当前连接发出的 QUIC 包数。
    pub sent_packets: u64,
    /// 当前连接判定丢失的 QUIC 包数。
    pub lost_packets: u64,
    /// 最近一次失败的原因。连上之后还留着，方便查刚才的问题。
    pub last_failure: Option<TunnelError>,
}

impl fmt::Display for TunnelStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Idle => f.write_str("未连接"),
            Self::Connecting => f.write_str("连接中"),
            Self::Connected => f.write_str("已连接"),
            Self::Disconnected(err) => write!(f, "已断开：{err}"),
            Self::Closed => f.write_str("已关闭"),
        }
    }
}

impl fmt::Display for PathKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Direct => "直连",
            Self::Relay => "中继",
        })
    }
}

/// 快直连的前台检查窗口。不使用 QUIC RTT：挂起期间积压的确认会把它拉高。
const REVALIDATE_DEADLINE: Duration = Duration::from_millis(1500);
/// ADR-0004 的历史中继往返在负载下到过 2.8 秒；未证实慢时不让死连接白等 8 秒。
const RELAY_REVALIDATE_DEADLINE: Duration = Duration::from_secs(3);
/// 连续两次主动检查都超过短窗口的路径，沿用定时 probe 的耐心。
const SLOW_REVALIDATE_DEADLINE: Duration = Duration::from_secs(8);
/// 一次建连最长这么久。n0 中继冷启动的 P90 约 5.5 秒（ADR-0004）；不设时要等 QUIC 的 30 秒超时，网络恢复时
/// 还卡在这次注定失败的握手上（ADR-0032 的改前实测：恢复后 32 秒才连上）。
const DIAL_DEADLINE: Duration = Duration::from_secs(12);
/// 请求等连接最长这么久，然后失败。比 [`DIAL_DEADLINE`] 长：刚发起的建连能等到真实结果。
const HOLD_DEADLINE: Duration = Duration::from_secs(15);

/// 到一台服务端的隧道客户端。克隆得到的是同一个客户端，共享同一条连接。
#[derive(Debug, Clone)]
pub struct TunnelClient {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    endpoint: Endpoint,
    server: Mutex<EndpointAddr>,
    /// 当前的建连尝试。同一时刻最多一次建连，并发请求共享它的结果。
    attempt: Mutex<Option<Arc<Attempt>>>,
    status: watch::Sender<TunnelStatus>,
    closed: AtomicBool,
    pacing: Mutex<Pacing>,
    stats: Mutex<TunnelStats>,
    /// 服务端确认过的连接：没确认就断开的（比如被拒绝）要退避，确认过的断开了可以马上重连。
    confirmed: Mutex<Option<usize>>,
    slow_path: Mutex<Option<SlowPath>>,
    clock: Option<Arc<dyn SuspendClock>>,
    suspension: Mutex<suspend::Suspension>,
}

#[derive(Debug, PartialEq, Eq)]
struct CheckedPath {
    conn_id: usize,
    remote: TransportAddr,
    relay: bool,
}

impl CheckedPath {
    fn selected(conn: &Connection) -> Option<Self> {
        let paths = conn.paths();
        let path = paths.iter().find(|path| path.is_selected())?;
        Some(Self {
            conn_id: conn.stable_id(),
            remote: path.remote_addr().clone(),
            relay: path.is_relay(),
        })
    }
}

#[derive(Debug)]
struct SlowPath {
    path: CheckedPath,
    samples: u8,
}

/// 隧道上的一条流，已写入请求类型。
#[derive(Debug)]
pub(crate) struct TunnelStream {
    pub(crate) send: SendStream,
    pub(crate) recv: RecvStream,
    /// 承载这条流的连接，用于在服务端回状态后把它标记为已授权。
    pub(crate) conn_id: usize,
}

impl TunnelStream {
    /// 复位流的两个方向。
    pub(crate) fn abort(&mut self) {
        forward::abort(&mut self.send, &mut self.recv);
    }
}

impl TunnelClient {
    /// 用已绑定的 endpoint 连接 `server`。不会立即建连，第一次使用时才连。
    ///
    /// `server` 可以只有 endpoint ID（靠 endpoint 配置的地址发现找到服务端），也可以带上中继地址和
    /// 直连地址。
    pub fn new(endpoint: Endpoint, server: impl Into<EndpointAddr>) -> Self {
        Self::with_clock(endpoint, server.into(), None)
    }

    /// 注入挂起检测的双时钟；不改变 QUIC 和主动检查的时限。
    #[doc(hidden)]
    pub fn with_suspend_clock(
        endpoint: Endpoint,
        server: impl Into<EndpointAddr>,
        clock: Arc<dyn SuspendClock>,
    ) -> Self {
        Self::with_clock(endpoint, server.into(), Some(clock))
    }

    fn with_clock(
        endpoint: Endpoint,
        server: EndpointAddr,
        clock: Option<Arc<dyn SuspendClock>>,
    ) -> Self {
        let (status, _) = watch::channel(TunnelStatus::Idle);
        Self {
            inner: Arc::new(Inner {
                endpoint,
                server: Mutex::new(server),
                attempt: Mutex::new(None),
                status,
                closed: AtomicBool::new(false),
                pacing: Mutex::default(),
                stats: Mutex::default(),
                confirmed: Mutex::new(None),
                slow_path: Mutex::new(None),
                clock,
                suspension: Mutex::default(),
            }),
        }
    }

    /// 服务端的 endpoint ID。
    pub fn server_id(&self) -> EndpointId {
        lock(&self.inner.server).id
    }

    /// 换服务端的地址，下一次建连时生效。服务器换了 IP 时，新地址由核心告诉 App（ADR-0032）。
    pub fn set_server_addr(&self, server: EndpointAddr) {
        *lock(&self.inner.server) = server;
    }

    /// 当前状态。
    pub fn status(&self) -> TunnelStatus {
        self.inner.status.borrow().clone()
    }

    /// 订阅状态变化。watch 只保留最新状态，订阅方处理慢也不会积压事件。
    pub fn watch_status(&self) -> watch::Receiver<TunnelStatus> {
        self.inner.status.subscribe()
    }

    /// 确认隧道可用：必要时建连，再和服务端做一次往返，确认本设备已获授权。
    ///
    /// 返回这次调用的总耗时；已有连接时约等于一个往返时间，需要建连时包含建连时间。
    pub async fn ping(&self) -> Result<Duration, TunnelError> {
        let started = Instant::now();
        let mut stream = self.open(Request::Ping).await?;
        stream.send.finish().ok();
        self.read_status(&mut stream.recv, stream.conn_id).await?;
        Ok(started.elapsed())
    }

    /// 确认现有连接仍然可用。App 从挂起中恢复（回到前台）时调用。
    ///
    /// 挂起超过路径的空闲超时后，服务端会放弃这条连接，关闭帧却到不了挂起的设备：不检查的话，要等本机的
    /// 空闲超时（iroh 默认 30 秒）才发现，期间所有请求都挂着。这里限时做一次往返，没有回应就关掉旧连接、
    /// 立刻建新连接。检查期间新开的流等检查结果，再走确认过的连接或新连接。没有连接时等同于 [`ping`]。
    ///
    /// [`ping`]: Self::ping
    pub async fn revalidate(&self) -> Result<Duration, TunnelError> {
        if self.inner.closed.load(Ordering::SeqCst) {
            return Err(TunnelError::Closed);
        }
        let started = Instant::now();
        let attempt = self.start_revalidation();
        let conn = match attempt.finished().await {
            Some(result) => result?,
            None => return self.ping().await,
        };
        if let Some(reason) = conn.close_reason() {
            return Err(from_connection_error(&reason));
        }
        if !attempt.revalidating {
            self.ping_on(&conn).await?;
        }
        self.inner.confirm(conn.stable_id());
        Ok(started.elapsed())
    }

    fn start_revalidation(&self) -> Arc<Attempt> {
        lock(&self.inner.pacing).reset();
        let mut current = lock(&self.inner.attempt);
        let mut live = None;
        if let Some(attempt) = current.as_ref() {
            match attempt.outcome() {
                None => return attempt.clone(),
                Some(Ok(conn)) if was_rejected(&conn) => return attempt.clone(),
                Some(Err(TunnelError::Unauthorized)) => return attempt.clone(),
                Some(result) => {
                    let usable = match &result {
                        Ok(conn) => conn.close_reason().is_none(),
                        Err(_) => true,
                    };
                    if usable
                        && attempt.recently_revalidated(suspend::now(self.inner.clock.as_deref()))
                    {
                        return attempt.clone();
                    }
                    live = result.ok().filter(|conn| conn.close_reason().is_none());
                }
            }
        }
        let attempt = if let Some(conn) = live {
            Attempt::spawn(
                false,
                true,
                &self.inner,
                self.clone().confirm_or_replace(conn),
            )
        } else {
            let client = self.clone();
            Attempt::spawn(true, true, &self.inner, async move {
                let conn = client.inner.clone().dial().await?;
                if let Err(err) = client.ping_on(&conn).await {
                    client
                        .inner
                        .set_status(TunnelStatus::Disconnected(err.clone()));
                    return Err(err);
                }
                Ok(conn)
            })
        };
        *current = Some(attempt.clone());
        attempt
    }

    /// 若现在 [`revalidate`]，会用多长的确认窗口。给测试把「慢路径的证据是否改变了决策」钉成确定性
    /// 断言用：窗口只取决于连接、路径和探测证据，与机器负载无关。
    ///
    /// [`revalidate`]: Self::revalidate
    #[doc(hidden)]
    pub fn next_revalidate_window(&self) -> Duration {
        self.inner
            .live_connection()
            .map_or(REVALIDATE_DEADLINE, |conn| self.revalidate_deadline(&conn))
    }

    /// 限时确认隧道真的通：有连接时在它上面做一次往返，没有回应就断开它，连接状态和路径随之更新；没有连接时
    /// 等同于 [`ping`]。
    ///
    /// QUIC 自己的判断不够：心跳和确认这些小包还能往返时，连接算活着，请求却可能过不去（#154）。和
    /// [`revalidate`] 不同，检查期间新开的流照常走现有连接，所以可以定时调用，不拖慢请求。
    ///
    /// [`ping`]: Self::ping
    /// [`revalidate`]: Self::revalidate
    pub async fn probe(&self, deadline: Duration) -> Result<Duration, TunnelError> {
        let Some(conn) = self.inner.live_connection() else {
            return self.ping().await;
        };
        let path = CheckedPath::selected(&conn);
        let started = Instant::now();
        match tokio::time::timeout(deadline, self.ping_on(&conn)).await {
            Ok(Ok(())) => {
                let elapsed = started.elapsed();
                if self.inner.is_current(conn.stable_id()) && path == CheckedPath::selected(&conn) {
                    self.record_checked_path(path, elapsed);
                }
                Ok(elapsed)
            }
            Ok(Err(err)) => Err(err),
            Err(_) => {
                info!(?deadline, "现有连接限时内没有回应，断开");
                let err = TunnelError::transport("连接限时内没有回应");
                // 先断开再写状态：订阅状态的人读路径时，这条连接已经不算了。
                conn.close(CLOSE_NORMAL, b"no response");
                self.inner
                    .mark(conn.stable_id(), TunnelStatus::Disconnected(err.clone()));
                Err(err)
            }
        }
    }

    fn record_checked_path(&self, path: Option<CheckedPath>, elapsed: Duration) {
        let mut proof = lock(&self.inner.slow_path);
        let Some(path) = path.filter(|_| elapsed >= REVALIDATE_DEADLINE) else {
            *proof = None;
            return;
        };
        match proof.as_mut() {
            Some(previous) if previous.path == path => {
                previous.samples = previous.samples.saturating_add(1).min(2);
            }
            _ => *proof = Some(SlowPath { path, samples: 1 }),
        }
    }

    fn revalidate_deadline(&self, conn: &Connection) -> Duration {
        let Some(path) = CheckedPath::selected(conn) else {
            return REVALIDATE_DEADLINE;
        };
        let slow = lock(&self.inner.slow_path);
        let proven_slow = slow
            .as_ref()
            .is_some_and(|proof| proof.path == path && proof.samples == 2);
        revalidation_window(path.relay, proven_slow)
    }

    /// 别等了，马上重试：清零退避，取消正在进行的建连。回到前台、网络变化、主人点“重试”时调用。
    ///
    /// 换网之后，旧网络上的握手已经没有意义，它的重传间隔也退避到了几秒。现有连接不动，要不要换由
    /// [`probe`](Self::probe) 确认：iroh 可能正在把它迁移到新路径上。
    pub fn retry_now(&self) {
        lock(&self.inner.pacing).reset();
        let mut current = lock(&self.inner.attempt);
        let replaceable = current
            .as_ref()
            .is_some_and(|attempt| match attempt.outcome() {
                None => attempt.cancellable,
                Some(Ok(conn)) => was_rejected(&conn),
                Some(Err(err)) => err == TunnelError::Unauthorized,
            });
        if replaceable && let Some(attempt) = current.take() {
            attempt.task.abort();
        }
    }

    /// 连接质量的指标。
    pub fn stats(&self) -> TunnelStats {
        let mut stats = lock(&self.inner.stats).clone();
        if let Some(conn) = self.inner.live_connection() {
            let current = conn.stats();
            stats.sent_packets = current.sent_packets;
            stats.lost_packets = current.lost_packets;
        }
        stats
    }

    /// 当前连接正在使用的传输路径。没有可用连接时返回 `None`。
    pub fn path(&self) -> Option<PathInfo> {
        let conn = self.inner.live_connection()?;
        let paths = conn.paths();
        let selected = paths.iter().find(|path| path.is_selected())?;
        let kind = if selected.is_relay() {
            PathKind::Relay
        } else {
            PathKind::Direct
        };
        Some(PathInfo {
            kind,
            remote: selected.remote_addr().clone(),
            rtt: selected.rtt(),
        })
    }

    /// 当前连接的路径变化；流不持有连接，连接关闭后结束。先订阅再取路径快照，避免漏掉切换。
    pub fn path_events(&self) -> Option<PathEventStream> {
        self.inner.live_connection().map(|conn| conn.path_events())
    }

    /// 关闭客户端：断开当前连接，之后的请求都返回 [`TunnelError::Closed`]。
    ///
    /// endpoint 归调用方所有，由调用方关闭。
    pub fn close(&self) {
        self.inner.closed.store(true, Ordering::SeqCst);
        self.set_suspendable(false);
        if let Some(conn) = self.inner.live_connection() {
            conn.close(CLOSE_NORMAL, b"client closed");
        }
        if let Some(attempt) = lock(&self.inner.attempt).take() {
            attempt.task.abort();
        }
        self.inner.status.send_replace(TunnelStatus::Closed);
    }

    /// 开一条流并写入请求类型，不等服务端回状态，调用方可以紧接着发送数据。
    pub(crate) async fn open(&self, request: Request) -> Result<TunnelStream, TunnelError> {
        self.inner.observe_resume(false);
        let mut retried = false;
        loop {
            let conn = self.connection().await?;
            match open_stream(&conn, request).await {
                Ok(stream) => return Ok(stream),
                // 连接恰好在检查之后断开。这时还没发出任何数据，换一条新连接重试一次是安全的。
                Err(err @ TunnelError::TransportUnavailable { .. }) if !retried => {
                    debug!("开流失败，换新连接重试：{err}");
                    retried = true;
                }
                Err(err) => return Err(err),
            }
        }
    }

    /// 读服务端在流上回的状态字节。服务端回了状态，说明它接受了这台设备。
    pub(crate) async fn read_status(
        &self,
        recv: &mut RecvStream,
        conn_id: usize,
    ) -> Result<(), TunnelError> {
        let mut byte = [0u8; 1];
        recv.read_exact(&mut byte)
            .await
            .map_err(|err| from_read_exact_error(&err))?;
        let status = Status::from_byte(byte[0])
            .ok_or_else(|| TunnelError::protocol(format!("未知的状态字节 {}", byte[0])))?;
        self.inner.confirm(conn_id);
        match status {
            Status::Ok => Ok(()),
            Status::UpstreamUnavailable => Err(TunnelError::UpstreamUnavailable),
            Status::UnsupportedRequest => Err(TunnelError::protocol("服务端不支持这个请求类型")),
            Status::PluginNotInstalled => Err(TunnelError::protocol("请求指定的插件没有安装")),
            Status::PortNotDeclared => Err(TunnelError::protocol("插件没有声明请求的端口和协议")),
            Status::PluginNotRunning => Err(TunnelError::protocol("请求指定的插件没有运行")),
        }
    }

    /// 限时在 `conn` 上做一次往返；没有回应或已经出错，就关掉它，建一条新连接。
    async fn confirm_or_replace(self, conn: Connection) -> Result<Connection, TunnelError> {
        let deadline = self.revalidate_deadline(&conn);
        match tokio::time::timeout(deadline, self.ping_on(&conn)).await {
            Ok(Ok(())) => return Ok(conn),
            Ok(Err(err)) => info!("现有连接不可用，重新建连：{err}"),
            Err(_) => info!(?deadline, "现有连接限时内没有回应，换新连接"),
        }
        conn.close(CLOSE_NORMAL, b"no response");
        let conn = self.inner.clone().dial().await?;
        if let Err(err) = self.ping_on(&conn).await {
            self.inner
                .set_status(TunnelStatus::Disconnected(err.clone()));
            return Err(err);
        }
        Ok(conn)
    }

    /// 在指定的连接上做一次往返，不经过 [`Self::connection`]。
    async fn ping_on(&self, conn: &Connection) -> Result<(), TunnelError> {
        let mut stream = open_stream(conn, Request::Ping).await?;
        stream.send.finish().ok();
        self.read_status(&mut stream.recv, stream.conn_id).await
    }

    /// 取一条可用连接：复用现有连接；没有或已断开时等下一次建连，最多等 [`HOLD_DEADLINE`]。
    async fn connection(&self) -> Result<Connection, TunnelError> {
        match tokio::time::timeout(HOLD_DEADLINE, self.wait_for_connection()).await {
            Ok(result) => result,
            Err(_) => Err(self
                .stats()
                .last_failure
                .unwrap_or_else(|| TunnelError::transport("连接限时内没有建好"))),
        }
    }

    async fn wait_for_connection(&self) -> Result<Connection, TunnelError> {
        loop {
            if self.inner.closed.load(Ordering::SeqCst) {
                return Err(TunnelError::Closed);
            }
            let attempt = self.inner.usable_attempt();
            // 被 `retry_now` 取消了：取新的那次。
            let Some(result) = attempt.finished().await else {
                continue;
            };
            let conn = result?;
            return match conn.close_reason() {
                None => Ok(conn),
                // 刚建好就被关闭。被服务端拒绝时，下一次调用也是这个结果；其他原因下一次调用会重新建连。
                Some(reason) => Err(from_connection_error(&reason)),
            };
        }
    }
}

impl Inner {
    /// 返回进行中或可用的建连尝试；没有、上一次已经失败或连接已断开时开始新的一次。
    fn usable_attempt(self: &Arc<Self>) -> Arc<Attempt> {
        let mut current = lock(&self.attempt);
        if let Some(attempt) = current.as_ref() {
            let stale = match attempt.outcome() {
                None => false,
                // 服务端不认这台设备时，重拨只会再被拒一次，状态还会在“连接中”和“要配对”之间跳。等 `retry_now`。
                Some(Ok(conn)) => conn.close_reason().is_some() && !was_rejected(&conn),
                Some(Err(TunnelError::Unauthorized)) => false,
                Some(Err(_)) => !attempt.recently_revalidated(suspend::now(self.clock.as_deref())),
            };
            if !stale {
                return attempt.clone();
            }
        }
        let attempt = Attempt::spawn(true, false, self, self.clone().dial());
        *current = Some(attempt.clone());
        attempt
    }

    fn live_connection(&self) -> Option<Connection> {
        let attempt = lock(&self.attempt).clone()?;
        match attempt.outcome() {
            Some(Ok(conn)) if conn.close_reason().is_none() => Some(conn),
            _ => None,
        }
    }

    /// 等到退避结束，建一次连接。
    async fn dial(self: Arc<Self>) -> Result<Connection, TunnelError> {
        let not_before = lock(&self.pacing).not_before();
        if let Some(not_before) = not_before {
            tokio::time::sleep_until(not_before).await;
        }
        self.set_status(TunnelStatus::Connecting);
        let server_addr = lock(&self.server).clone();
        let server = server_addr.id.fmt_short();
        let started = Instant::now();
        let dialed =
            tokio::time::timeout(DIAL_DEADLINE, self.endpoint.connect(server_addr, ALPN)).await;
        let conn = match dialed {
            Ok(Ok(conn)) => conn,
            Ok(Err(err)) => return Err(self.dial_failed(from_connect_error(&err), &server)),
            Err(_) => {
                let err = TunnelError::transport("建连限时内没有回应");
                return Err(self.dial_failed(err, &server));
            }
        };
        if self.closed.load(Ordering::SeqCst) {
            conn.close(CLOSE_NORMAL, b"client closed");
            return Err(TunnelError::Closed);
        }
        let handshake = started.elapsed();
        lock(&self.stats).handshake = Some(handshake);
        let handshake_ms = handshake.as_millis();
        info!(%server, handshake_ms, "隧道连接已建立");
        watch_close(Arc::downgrade(&self), &conn);
        Ok(conn)
    }

    fn dial_failed(&self, err: TunnelError, server: &impl fmt::Display) -> TunnelError {
        info!(%server, "隧道建连失败：{err}");
        lock(&self.pacing).failed();
        self.set_status(TunnelStatus::Disconnected(err.clone()));
        err
    }

    /// 服务端回应了这条连接：确认本设备被接受。
    fn confirm(&self, conn_id: usize) {
        if !self.is_current(conn_id) {
            return;
        }
        *lock(&self.confirmed) = Some(conn_id);
        lock(&self.pacing).reset();
        lock(&self.stats).last_confirmed = Some(SystemTime::now());
        self.set_status(TunnelStatus::Connected);
    }

    fn is_current(&self, conn_id: usize) -> bool {
        let attempt = lock(&self.attempt).clone();
        attempt.is_some_and(
            |attempt| matches!(attempt.outcome(), Some(Ok(conn)) if conn.stable_id() == conn_id),
        )
    }

    /// 只有 `conn_id` 仍是当前连接时才更新状态，避免旧连接的事件覆盖新连接的状态。
    fn mark(&self, conn_id: usize, status: TunnelStatus) {
        if self.is_current(conn_id) {
            self.set_status(status);
        }
    }

    /// 更新状态；客户端关闭后状态固定为 `Closed`。
    fn set_status(&self, status: TunnelStatus) {
        self.status.send_if_modified(|current| {
            if *current == TunnelStatus::Closed || *current == status {
                return false;
            }
            let mut stats = lock(&self.stats);
            if *current == TunnelStatus::Connected {
                stats.disconnects = stats.disconnects.saturating_add(1);
            }
            if let TunnelStatus::Disconnected(err) = &status {
                stats.last_failure = Some(err.clone());
            }
            *current = status;
            true
        });
    }
}

fn was_rejected(conn: &Connection) -> bool {
    conn.close_reason()
        .is_some_and(|reason| matches!(from_connection_error(&reason), TunnelError::Unauthorized))
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// 监听连接关闭，把断开原因写进状态。只持有弱引用，不会让连接或客户端多活。
fn watch_close(inner: Weak<Inner>, conn: &Connection) {
    let closed = conn.weak_handle().closed();
    let conn_id = conn.stable_id();
    tokio::spawn(async move {
        let Some(closed) = closed.await else {
            return;
        };
        let Some(inner) = inner.upgrade() else {
            return;
        };
        // 本机断开的连接，断开的地方已经写了状态和原因。
        if matches!(closed.reason, ConnectionError::LocallyClosed) {
            return;
        }
        let err = from_connection_error(&closed.reason);
        let server = lock(&inner.server).id.fmt_short();
        info!(%server, "隧道连接已断开：{err}");
        let confirmed = *lock(&inner.confirmed) == Some(conn_id);
        if !confirmed {
            lock(&inner.pacing).failed();
        }
        inner.mark(conn_id, TunnelStatus::Disconnected(err));
    });
}

async fn open_stream(conn: &Connection, request: Request) -> Result<TunnelStream, TunnelError> {
    let (mut send, recv) = conn
        .open_bi()
        .await
        .map_err(|err| from_connection_error(&err))?;
    send.write_all(&[request as u8])
        .await
        .map_err(|err| from_write_error(&err))?;
    Ok(TunnelStream {
        send,
        recv,
        conn_id: conn.stable_id(),
    })
}

fn revalidation_window(relay: bool, proven_slow: bool) -> Duration {
    match (relay, proven_slow) {
        (_, true) => SLOW_REVALIDATE_DEADLINE,
        (true, false) => RELAY_REVALIDATE_DEADLINE,
        (false, false) => REVALIDATE_DEADLINE,
    }
}

#[cfg(test)]
mod tests {
    use super::revalidation_window;
    use std::time::Duration;

    #[test]
    fn revalidation_balances_dead_relay_recovery_and_proven_slow_paths() {
        for (relay, slow, millis) in [
            (false, false, 1500),
            (true, false, 3000),
            (false, true, 8000),
            (true, true, 8000),
        ] {
            assert_eq!(
                revalidation_window(relay, slow),
                Duration::from_millis(millis)
            );
        }
    }
}
