//! 隧道客户端：所有请求复用一条到服务端的连接；连接断开后，下一次使用时自动重连。
//!
//! 同一时刻最多一次建连，失败后按 [`Pacing`] 退避，等待期间的请求共享下一次的结果（ADR-0032）。建连放在独立的
//! 任务里：等它的请求超时走了，建连也不会被丢下重来。

mod attempt;
mod check;
mod path;
mod ports;
pub(crate) mod streams;
mod suspend;

use attempt::Attempt;
use check::SlowPath;
pub use path::{PathInfo, PathKind};
pub use streams::TunnelStreamStats;
use streams::{StreamCounters, StreamLease, open_stream};
pub use suspend::SuspendClock;

use std::{
    fmt,
    sync::{
        Arc, Mutex, MutexGuard, PoisonError, Weak,
        atomic::{AtomicBool, AtomicU32, Ordering},
    },
    time::{Duration, Instant, SystemTime},
};

use iroh::{
    Endpoint, EndpointAddr, EndpointId,
    endpoint::{Connection, ConnectionError, PathEventStream, RecvStream, SendStream},
};
use tokio::sync::{Notify, watch};
use tracing::{debug, info};

use crate::{
    error::{TunnelError, from_connect_error, from_connection_error, from_read_exact_error},
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
    /// 当前连接收到的 QUIC 包数：判活用确认帧，这只是给日志和测试用的。
    pub received_packets: u64,
    /// 当前连接判定丢失的 QUIC 包数。
    pub lost_packets: u64,
    /// 距上次确认连上，建连已经失败的次数，连上就清零：判断“这一轮是不是已经失败过”用。给人看的
    /// 失败原因在 [`Self::last_failure`]，那边连上之后还留着。
    pub failed_dials: u32,
    /// 最近一次失败的原因。连上之后还留着，方便查刚才的问题。
    pub last_failure: Option<TunnelError>,
    /// 客户端开着的隧道流。
    pub streams: TunnelStreamStats,
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
///
/// 冷启动的慢设备上一次握手要更久（翻译层、冷缓存），首批建连由调用方用
/// [`TunnelClient::set_dial_deadline`] 放宽；之后的建连还是这个值，服务器真没了照样 12 秒就放弃。
pub const DIAL_DEADLINE: Duration = Duration::from_secs(12);
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
    /// 这台客户端建连用的 ALPN。隧道的默认 [`ALPN`]，联邦用 [`crate::federation::FEDERATION_ALPN`]。
    alpn: &'static [u8],
    server: Mutex<EndpointAddr>,
    /// 当前的建连尝试。同一时刻最多一次建连，并发请求共享它的结果。
    attempt: Mutex<Option<Arc<Attempt>>>,
    status: watch::Sender<TunnelStatus>,
    closed: AtomicBool,
    pacing: Mutex<Pacing>,
    stats: Mutex<TunnelStats>,
    streams: Arc<StreamCounters>,
    /// 服务端确认过的连接：没确认就断开的（比如被拒绝）要退避，确认过的断开了可以马上重连。
    confirmed: Mutex<Option<usize>>,
    /// 退避期间的新请求立即失败，不等正在退避的那次建连。设备隧道保持默认（等，最多
    /// [`HOLD_DEADLINE`]）；联邦出站打开它，见 [`TunnelClient::set_fail_fast_during_backoff`]。
    fail_fast: AtomicBool,
    slow_path: Mutex<Option<SlowPath>>,
    clock: Option<Arc<dyn SuspendClock>>,
    suspension: Mutex<suspend::Suspension>,
    /// 当前连接的数据报分发器，第一次开 UDP 流向时建；连接换了就换。
    datagrams: Mutex<Option<Arc<crate::udp::mux::DatagramMux>>>,
    /// 请求在隧道流上迟迟等不到服务端回执时催一下定时检查：立即确认一次，不等下一个检查周期
    /// 存一个许可，催多次合成一次。
    check_nudge: Notify,
    /// 建连时等多久：前几次（冷启动）用 `cold`，之后用 `warm`。
    dial: Mutex<DialTiming>,
}

/// 建连时限。默认宽窄都是 [`DIAL_DEADLINE`]，冷启动由调用方另设。
#[derive(Debug)]
struct DialTiming {
    /// 调用方手里还剩几次冷启动的建连，每次建连用掉一次。同一次建连可以拨好几次（断了重拨），
    /// 名额跨客户端共享，客户端换掉也不作废。
    allowance: Arc<AtomicU32>,
    cold: Duration,
    warm: Duration,
}

impl Default for DialTiming {
    fn default() -> Self {
        Self {
            allowance: Arc::new(AtomicU32::new(0)),
            cold: DIAL_DEADLINE,
            warm: DIAL_DEADLINE,
        }
    }
}

impl DialTiming {
    /// 这一次建连等多久。冷启动的名额由 [`take`] 扣，扣完用 `warm`。
    fn next(&self) -> Duration {
        if take(&self.allowance) {
            return self.cold;
        }
        self.warm
    }

    /// 已经连上过：剩下的名额作废。名额只给这一轮第一次连上之前的拨号，之后断了重拨按正常的时限。
    fn connected(&self) {
        self.allowance.store(0, Ordering::SeqCst);
    }
}

/// 名额还有就扣掉一个并返回真。
fn take(allowance: &AtomicU32) -> bool {
    let mut remaining = allowance.load(Ordering::SeqCst);
    while remaining > 0 {
        match allowance.compare_exchange_weak(
            remaining,
            remaining - 1,
            Ordering::SeqCst,
            Ordering::SeqCst,
        ) {
            Ok(_) => return true,
            Err(actual) => remaining = actual,
        }
    }
    false
}

/// 隧道上的一条流，已写入请求类型。
#[derive(Debug)]
pub(crate) struct TunnelStream {
    pub(crate) send: SendStream,
    pub(crate) recv: RecvStream,
    /// 承载这条流的连接，用于在服务端回状态后把它标记为已授权。
    pub(crate) conn_id: usize,
    pub(crate) _lease: StreamLease,
}

impl TunnelStream {
    /// 这条流所属客户端的计数：判活看它记下的新数据。
    pub(crate) fn counters(&self) -> &Arc<StreamCounters> {
        self._lease.counters()
    }

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
        Self::with_clock(endpoint, server.into(), ALPN, None)
    }

    /// ALPN 不同的调用方（如联邦，[`crate::federation::FEDERATION_ALPN`]）用它换协议名；重连、退避、
    /// 探活都和 [`Self::new`] 的一样。
    pub fn with_alpn(
        endpoint: Endpoint,
        server: impl Into<EndpointAddr>,
        alpn: &'static [u8],
    ) -> Self {
        Self::with_clock(endpoint, server.into(), alpn, None)
    }

    /// 注入挂起检测的双时钟；不改变 QUIC 和主动检查的时限。
    #[doc(hidden)]
    pub fn with_suspend_clock(
        endpoint: Endpoint,
        server: impl Into<EndpointAddr>,
        clock: Arc<dyn SuspendClock>,
    ) -> Self {
        Self::with_clock(endpoint, server.into(), ALPN, Some(clock))
    }

    fn with_clock(
        endpoint: Endpoint,
        server: EndpointAddr,
        alpn: &'static [u8],
        clock: Option<Arc<dyn SuspendClock>>,
    ) -> Self {
        let (status, _) = watch::channel(TunnelStatus::Idle);
        Self {
            inner: Arc::new(Inner {
                endpoint,
                alpn,
                server: Mutex::new(server),
                attempt: Mutex::new(None),
                status,
                closed: AtomicBool::new(false),
                pacing: Mutex::default(),
                stats: Mutex::default(),
                streams: Arc::default(),
                confirmed: Mutex::new(None),
                fail_fast: AtomicBool::new(false),
                slow_path: Mutex::new(None),
                clock,
                suspension: Mutex::default(),
                datagrams: Mutex::default(),
                check_nudge: Notify::new(),
                dial: Mutex::default(),
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

    /// 建连的时限：`allowance` 里还有名额时等 `cold`，用完之后等 `warm`。名额每次建连扣一个，是调用方
    /// 手里的计数——同一条连接断了重拨也不重新给。冷启动的慢设备（首次启动、冷缓存、x86 模拟器上跑翻译后的
    /// arm64 包）上一次握手比 [`DIAL_DEADLINE`] 久得多，首批建连要放宽；用完这批还是按 `warm` 放弃，
    /// 服务器真没了不会变慢。不设时全部用 [`DIAL_DEADLINE`]。
    pub fn set_dial_deadline(&self, allowance: Arc<AtomicU32>, cold: Duration, warm: Duration) {
        *lock(&self.inner.dial) = DialTiming {
            allowance,
            cold,
            warm,
        };
    }

    /// 退避期间的新请求立即失败（transport 错误），不共用正在退避的那次建连的结果。默认关闭：
    /// 等当前尝试，最多 [`HOLD_DEADLINE`]（ADR-0032）。联邦出站打开它：本机 Tuwunel 的联邦请求
    /// 自己会重试，替它等只会把 CONNECT 挂满退避。第一次使用前调用。
    pub fn set_fail_fast_during_backoff(&self) {
        self.inner.fail_fast.store(true, Ordering::Relaxed);
    }

    /// 当前状态。
    pub fn status(&self) -> TunnelStatus {
        self.inner.status.borrow().clone()
    }

    /// 订阅状态变化。watch 只保留最新状态，订阅方处理慢也不会积压事件。
    pub fn watch_status(&self) -> watch::Receiver<TunnelStatus> {
        self.inner.status.subscribe()
    }

    /// 催一下定时检查：请求在隧道流上迟迟等不到服务端回执时，立即确认一次连接，不等下一个
    /// 检查周期。确认的时限和判死证据与定时检查相同，不会因为催得早而误判慢中继。
    /// 没人在等时存一个许可，下一次等待马上返回。
    pub fn nudge_check(&self) {
        self.inner.check_nudge.notify_one();
    }

    /// 等下一次 [`Self::nudge_check`]。定时检查的循环和睡眠一起等它。
    pub async fn wait_for_check_nudge(&self) {
        self.inner.check_nudge.notified().await;
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
        stats.streams = self.inner.streams.snapshot();
        if let Some(conn) = self.inner.live_connection() {
            let current = conn.stats();
            stats.sent_packets = current.sent_packets;
            stats.lost_packets = current.lost_packets;
            stats.received_packets = current.udp_rx.datagrams;
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
        let stats = selected.stats();
        Some(PathInfo {
            connection_id: conn.stable_id(),
            kind,
            remote: selected.remote_addr().clone(),
            rtt: stats.rtt,
            sent_packets: stats.sent_packets,
            lost_packets: stats.lost_packets,
            sent_bytes: stats.sent_bytes,
            lost_bytes: stats.lost_bytes,
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
        // 打开了 fail-fast 的调用方（联邦出站）：退避期间立刻失败，调用方自己会重试。建连正在进行
        // （不在退避）时照常共用它的结果（ADR-0032）；有连接时照常在现有连接上开流。
        if self.inner.fail_fast.load(Ordering::Relaxed)
            && self.inner.live_connection().is_none()
            && lock(&self.inner.pacing)
                .not_before()
                .is_some_and(|not_before| not_before > tokio::time::Instant::now())
        {
            return Err(TunnelError::transport("建连在退避中"));
        }
        let mut retried = false;
        loop {
            let conn = self.connection().await?;
            match open_stream(&conn, request, &self.inner.streams).await {
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
        self.inner.streams.note_received(1);
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

    /// 在指定的连接上做一次往返，不经过 [`Self::connection`]。
    async fn ping_on(&self, conn: &Connection) -> Result<(), TunnelError> {
        let mut stream = open_stream(conn, Request::Ping, &self.inner.streams).await?;
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
            // 连接已经被对端关掉、`watch_close` 还没轮到写状态时，先如实写断开再重拨。不写的话状态从“已连接”
            // 直接跳到新一次建连的“连接中”，旧连接的关闭事件到时已不是当前连接、不再写：断开次数少记一次，
            // 核心也不知道这条连接是断了（实测请求恰好在关闭后立刻到达时，连不上的闸门放过了它，白等一次建连）。
            // 本机关掉的连接由关的地方写。
            if let Some(Ok(conn)) = attempt.outcome()
                && let Some(reason) = conn.close_reason()
                && !matches!(reason, ConnectionError::LocallyClosed)
            {
                self.set_status(TunnelStatus::Disconnected(from_connection_error(&reason)));
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
        let deadline = lock(&self.dial).next();
        let dialed =
            tokio::time::timeout(deadline, self.endpoint.connect(server_addr, self.alpn)).await;
        let conn = match dialed {
            Ok(Ok(conn)) => conn,
            Ok(Err(err)) => {
                let err = from_connect_error(&err);
                return Err(self.dial_failed(err, &server, deadline));
            }
            Err(_) => {
                let err = TunnelError::transport("建连限时内没有回应");
                return Err(self.dial_failed(err, &server, deadline));
            }
        };
        lock(&self.dial).connected();
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

    /// 记下这次失败。`deadline` 也记日志：查日志时看得出是哪个时限掐的（冷启动和正常的不一样）。
    fn dial_failed(
        &self,
        err: TunnelError,
        server: &impl fmt::Display,
        deadline: Duration,
    ) -> TunnelError {
        info!(%server, ?deadline, "隧道建连失败：{err}");
        lock(&self.pacing).failed();
        let mut stats = lock(&self.stats);
        stats.failed_dials = stats.failed_dials.saturating_add(1);
        drop(stats);
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
        let mut stats = lock(&self.stats);
        stats.last_confirmed = Some(SystemTime::now());
        stats.failed_dials = 0;
        drop(stats);
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

fn revalidation_window(relay: bool, proven_slow: bool) -> Duration {
    match (relay, proven_slow) {
        (_, true) => SLOW_REVALIDATE_DEADLINE,
        (true, false) => RELAY_REVALIDATE_DEADLINE,
        (false, false) => REVALIDATE_DEADLINE,
    }
}

#[cfg(test)]
mod tests {
    use super::{DialTiming, revalidation_window};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::Duration;

    /// 冷启动的名额按次用完：用完以后就是正常的时限。
    #[test]
    fn the_cold_allowance_runs_out_after_its_count() {
        let timing = DialTiming {
            allowance: Arc::new(AtomicU32::new(2)),
            cold: Duration::from_secs(30),
            warm: Duration::from_secs(12),
        };
        assert_eq!(timing.next(), Duration::from_secs(30));
        assert_eq!(timing.next(), Duration::from_secs(30));
        assert_eq!(timing.next(), Duration::from_secs(12));
    }

    /// 连上过一次就把剩下的名额作废：断线重拨时握手已经是热的，不该再用冷启动的耐心。
    #[test]
    fn a_connected_line_spends_the_remaining_allowance() {
        let allowance = Arc::new(AtomicU32::new(2));
        let timing = DialTiming {
            allowance: allowance.clone(),
            cold: Duration::from_secs(30),
            warm: Duration::from_secs(12),
        };
        assert_eq!(timing.next(), Duration::from_secs(30));
        timing.connected();
        assert_eq!(allowance.load(Ordering::SeqCst), 0, "名额清零");
        assert_eq!(timing.next(), Duration::from_secs(12));
    }

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
