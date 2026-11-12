//! 到一台对方服务器的出站连接：所有流复用一条连接，断开后下一次使用时按退避重连，定时探活。
//!
//! 接口对齐 [`TunnelClient`](crate::TunnelClient)（共享连接、退避、探活、按连接确认授权）。不直接用它
//! 的原因：它的 ALPN 固定为隧道的，而隧道负责人正在改客户端（#506），合并后这里整体换成 `TunnelClient`。

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::{Arc, Mutex, MutexGuard, PoisonError, Weak},
    time::{Duration, Instant, SystemTime},
};

use iroh::{
    Endpoint, EndpointAddr, EndpointId,
    endpoint::{Connection, ConnectionError, RecvStream, SendStream},
};
use tokio::{sync::watch, task::AbortHandle};
use tracing::{debug, info};

use crate::{
    error::{
        TunnelError, from_connect_error, from_connection_error, from_read_exact_error,
        from_write_error,
    },
    federation::{FEDERATION_ALPN, ServerLookup},
    forward,
    pacing::Pacing,
    wire::{CLOSE_NORMAL, Request, Status},
};

/// 一次建连最长这么久。取值和隧道客户端一致：n0 中继冷启动的 P90 约 5.5 秒（ADR-0004）。
const DIAL_DEADLINE: Duration = Duration::from_secs(12);

/// 一台对方服务器的连接状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PeerStatus {
    /// 最近一次探活或数据往返成功。
    Connected,
    /// 连不上对方：`since` 是变成离线的时刻，`last_error` 是最近一次失败的原因。下一次使用或下一次
    /// 探活时按退避自动重连。
    Offline {
        since: SystemTime,
        last_error: TunnelError,
    },
    /// 握手后被对方用 403 关闭：对方的服务器名单里没有本机（没配对，或配对已被对方删除）。
    /// 对方的名单恢复后，下一次探活会自动变回 [`PeerStatus::Connected`]。
    RemovedByPeer,
}

/// 已接触过的各台对方服务器的状态，按 Matrix 服务器名索引。
///
/// 名字还没出现 = 还没有这台服务器的探活结果（刚配对时）；名字从配对名单里删除后，条目也随之消失。
pub type PeerStates = BTreeMap<String, PeerStatus>;

/// 出站侧的一条转发流，已写入请求类型。
pub(crate) struct ForwardStream {
    pub(crate) send: SendStream,
    pub(crate) recv: RecvStream,
    /// 承载这条流的连接，用来在对方回状态后确认这条连接可用。
    pub(crate) conn_id: usize,
}

impl ForwardStream {
    /// 复位流的两个方向。
    pub(crate) fn abort(&mut self) {
        forward::abort(&mut self.send, &mut self.recv);
    }
}

/// 出站侧按服务器名跟踪的对方服务器：每台一个 [`PeerClient`] 和一个探活任务。
#[derive(Debug, Default)]
pub(crate) struct PeerTable {
    peers: Mutex<HashMap<String, Tracked>>,
}

#[derive(Debug)]
struct Tracked {
    client: Arc<PeerClient>,
    ping: AbortHandle,
}

impl PeerTable {
    /// 取这台对方服务器的客户端，没有就建一个，并开始定时探活。
    pub(crate) fn get_or_create(
        self: &Arc<Self>,
        name: &str,
        id: EndpointId,
        addr: EndpointAddr,
        ctx: &super::outbound::ForwarderCtx,
    ) -> Arc<PeerClient> {
        let mut peers = lock(&self.peers);
        if let Some(tracked) = peers.get(name) {
            return tracked.client.clone();
        }
        let client = PeerClient::new(
            name.to_owned(),
            id,
            addr,
            ctx.endpoint.clone(),
            ctx.status.clone(),
        );
        // 先占位再起探活任务：探活任务拆掉自己（remove）时一定找得到条目。
        let ping = tokio::spawn(ping_loop(
            client.clone(),
            self.clone(),
            ctx.lookup.clone(),
            ctx.ping_interval,
            ctx.ping_timeout,
        ))
        .abort_handle();
        peers.insert(
            name.to_owned(),
            Tracked {
                client: client.clone(),
                ping,
            },
        );
        client
    }

    /// 拆掉一台对方服务器：停探活、断连接、状态条目删掉。
    pub(crate) fn remove(self: &Arc<Self>, name: &str) {
        let Some(tracked) = lock(&self.peers).remove(name) else {
            return;
        };
        tracked.ping.abort();
        tracked.client.hang_up();
        tracked.client.remove_state();
    }

    /// 拆掉不在 `names` 里的每一台，留下的不动。
    pub(crate) fn retain(self: &Arc<Self>, names: &BTreeSet<String>) {
        let gone: Vec<String> = lock(&self.peers)
            .keys()
            .filter(|name| !names.contains(*name))
            .cloned()
            .collect();
        for name in gone {
            self.remove(&name);
        }
    }

    /// 停掉所有探活任务并断开连接。转发器停止时调用。
    pub(crate) fn stop_all(&self) {
        for (_, tracked) in lock(&self.peers).drain() {
            tracked.ping.abort();
            tracked.client.hang_up();
        }
    }
}

/// 到一台对方服务器的连接。引用共享同一条连接、退避和状态。
#[derive(Debug)]
pub(crate) struct PeerClient {
    /// 对方的 Matrix 服务器名，也是状态表里的键。服务器名和 endpoint ID 的绑定只在配对时定（ADR-0050）。
    name: String,
    endpoint: Endpoint,
    /// 出站要连的对方 endpoint ID 和地址。每次连接前由出站侧和探活任务用最新的查找结果更新。
    target: Mutex<(EndpointId, EndpointAddr)>,
    /// 当前的连接。`dial_lock` 保证同一时刻最多一次建连，等它的请求共享结果。
    conn: Mutex<Option<Connection>>,
    /// 对方确认过的连接：没确认就断开的（比如被拒绝）要退避，确认过的断开了可以马上重连。
    confirmed: Mutex<Option<usize>>,
    dial_lock: tokio::sync::Mutex<()>,
    pacing: Mutex<Pacing>,
    status: watch::Sender<PeerStates>,
}

impl PeerClient {
    fn new(
        name: String,
        id: EndpointId,
        addr: EndpointAddr,
        endpoint: Endpoint,
        status: watch::Sender<PeerStates>,
    ) -> Arc<Self> {
        Arc::new(Self {
            name,
            endpoint,
            target: Mutex::new((id, addr)),
            conn: Mutex::new(None),
            confirmed: Mutex::new(None),
            dial_lock: tokio::sync::Mutex::new(()),
            pacing: Mutex::new(Pacing::default()),
            status,
        })
    }

    /// 更新对方的 endpoint ID 和地址（来自最新的配对信息）。endpoint ID 换了（重新配对）时，旧连接作废。
    pub(crate) fn set_target(&self, id: EndpointId, addr: EndpointAddr) {
        let mut target = lock(&self.target);
        if target.0 != id
            && let Some(conn) = lock(&self.conn).take()
        {
            conn.close(CLOSE_NORMAL, b"peer rebound");
            *lock(&self.confirmed) = None;
        }
        *target = (id, addr);
    }

    /// 开一条转发流：复用现有连接，或建一条新连接（受退避约束）。
    pub(crate) async fn open_forward(self: &Arc<Self>) -> Result<ForwardStream, TunnelError> {
        let mut retried = false;
        loop {
            let conn = self.connection().await?;
            match conn.open_bi().await {
                Ok((mut send, recv)) => {
                    if let Err(err) = send.write_all(&[Request::Forward as u8]).await {
                        return Err(from_write_error(&err));
                    }
                    return Ok(ForwardStream {
                        send,
                        recv,
                        conn_id: conn.stable_id(),
                    });
                }
                // 连接恰好在检查之后断开。这时还没发出任何数据，换一条新连接重试一次是安全的。
                Err(err) if !retried => {
                    debug!(server = %self.name, "opening the stream failed, retrying with a new connection: {err}");
                    retried = true;
                }
                Err(err) => return Err(from_connection_error(&err)),
            }
        }
    }

    /// 限时确认到对方的连接真的通：有连接时在它上面做一次 ping 往返，没有回应就关掉它。没有连接时
    /// 建一条再探；还在退避时不试，返回 `Ok(None)`，等下个周期。建连有自己的上限，`timeout` 只管探活
    /// 这一次往返。
    pub(crate) async fn probe(
        self: &Arc<Self>,
        timeout: Duration,
    ) -> Result<Option<Duration>, TunnelError> {
        let conn = match self.live_connection() {
            Some(conn) => conn,
            None if self.backoff_pending() => return Ok(None),
            None => self.connection().await?,
        };
        let started = Instant::now();
        match tokio::time::timeout(timeout, ping_on(&conn)).await {
            Ok(Ok(())) => {
                self.note_confirmed(conn.stable_id());
                Ok(Some(started.elapsed()))
            }
            Ok(Err(err)) => Err(err),
            Err(_) => {
                info!(server = %self.name, ?timeout, "the existing connection did not respond in time, closing it");
                conn.close(CLOSE_NORMAL, b"no response");
                Err(TunnelError::transport("the probe did not respond in time"))
            }
        }
    }

    fn backoff_pending(&self) -> bool {
        match lock(&self.pacing).not_before() {
            Some(at) => tokio::time::Instant::now() < at,
            None => false,
        }
    }

    /// 下一次探活要等多久：有连接按周期；没有连接时按退避，没在退避就立刻试。
    pub(crate) fn next_probe_wait(&self, interval: Duration) -> Duration {
        if self.live_connection().is_some() {
            return interval;
        }
        match lock(&self.pacing).not_before() {
            Some(at) => at.saturating_duration_since(tokio::time::Instant::now()),
            None => Duration::ZERO,
        }
    }

    /// 探活结果落状态。`Ok(None)` 是这次没探（还在退避），状态不动。
    pub(crate) fn settle(&self, result: Result<Option<Duration>, TunnelError>) {
        match result {
            Ok(Some(_)) => {
                lock(&self.pacing).reset();
                self.set_status(PeerStatus::Connected);
            }
            Ok(None) => {}
            Err(TunnelError::Unauthorized) => {
                lock(&self.pacing).failed();
                self.set_status(PeerStatus::RemovedByPeer);
            }
            Err(err) => {
                lock(&self.pacing).failed();
                self.set_offline(err);
            }
        }
    }

    /// 数据流上对方回过状态：这条连接确实是通的。
    pub(crate) fn note_confirmed(&self, conn_id: usize) {
        if !self.is_current(conn_id) {
            return;
        }
        *lock(&self.confirmed) = Some(conn_id);
        lock(&self.pacing).reset();
        self.set_status(PeerStatus::Connected);
    }

    /// 关掉缓存的连接（本机主动关闭，关闭事件不写状态）。
    fn hang_up(&self) {
        if let Some(conn) = lock(&self.conn).take() {
            conn.close(CLOSE_NORMAL, b"peer removed");
        }
        *lock(&self.confirmed) = None;
    }

    /// 名字已从配对名单里删除，状态条目随之消失。
    fn remove_state(&self) {
        self.status
            .send_if_modified(|states| states.remove(&self.name).is_some());
    }

    /// 取一条可用连接：复用现有连接；没有时建一条。退避没到点时直接失败，下一次使用或探活再试。
    async fn connection(self: &Arc<Self>) -> Result<Connection, TunnelError> {
        let _guard = self.dial_lock.lock().await;
        if let Some(conn) = self.live_connection() {
            return Ok(conn);
        }
        if let Some(not_before) = lock(&self.pacing).not_before()
            && tokio::time::Instant::now() < not_before
        {
            return Err(TunnelError::transport(
                "waiting for the backoff before reconnecting",
            ));
        }
        let (_, addr) = lock(&self.target).clone();
        let started = Instant::now();
        let dialed =
            tokio::time::timeout(DIAL_DEADLINE, self.endpoint.connect(addr, FEDERATION_ALPN)).await;
        let conn = match dialed {
            Ok(Ok(conn)) => conn,
            Ok(Err(err)) => return Err(self.dial_failed(from_connect_error(&err))),
            Err(_) => {
                return Err(self.dial_failed(TunnelError::transport(
                    "connecting did not complete in time",
                )));
            }
        };
        info!(
            server = %self.name,
            handshake_ms = started.elapsed().as_millis(),
            "federation connection established"
        );
        watch_close(Arc::downgrade(self), &conn);
        *lock(&self.conn) = Some(conn.clone());
        Ok(conn)
    }

    fn dial_failed(&self, err: TunnelError) -> TunnelError {
        info!(server = %self.name, "federation connection failed: {err}");
        // 状态由探活任务写：它下一次（按这里的退避）探活会试同一个名字，把真实原因记进状态。
        lock(&self.pacing).failed();
        err
    }

    fn live_connection(&self) -> Option<Connection> {
        lock(&self.conn)
            .clone()
            .filter(|conn| conn.close_reason().is_none())
    }

    fn is_current(&self, conn_id: usize) -> bool {
        lock(&self.conn)
            .as_ref()
            .is_some_and(|conn| conn.stable_id() == conn_id)
    }

    fn set_offline(&self, err: TunnelError) {
        self.status.send_if_modified(|states| {
            // `since` 保持第一次变成离线的时刻；错误刷新成最近一次的。
            let since = match states.get(&self.name) {
                Some(PeerStatus::Offline { since, .. }) => *since,
                _ => SystemTime::now(),
            };
            let next = PeerStatus::Offline {
                since,
                last_error: err,
            };
            if states.get(&self.name) == Some(&next) {
                return false;
            }
            states.insert(self.name.clone(), next);
            true
        });
    }

    fn set_status(&self, status: PeerStatus) {
        self.status.send_if_modified(|states| {
            if states.get(&self.name) == Some(&status) {
                return false;
            }
            states.insert(self.name.clone(), status);
            true
        });
    }

    /// 连接被对方（或传输）关闭时写状态。
    fn note_broken(&self, conn_id: usize, err: TunnelError) {
        if !self.is_current(conn_id) {
            return;
        }
        if *lock(&self.confirmed) != Some(conn_id) {
            lock(&self.pacing).failed();
        }
        if err == TunnelError::Unauthorized {
            info!(server = %self.name, "the peer server rejected this server");
            self.set_status(PeerStatus::RemovedByPeer);
        } else {
            info!(server = %self.name, "federation connection lost: {err}");
            self.set_offline(err);
        }
    }
}

/// 定时探活一台对方服务器。名字从查找函数里消失（对方已从配对名单里删除）时，把它整个拆掉。
async fn ping_loop(
    client: Arc<PeerClient>,
    peers: Arc<PeerTable>,
    lookup: ServerLookup,
    interval: Duration,
    timeout: Duration,
) {
    loop {
        match lookup(&client.name) {
            Some((id, addr)) => client.set_target(id, addr),
            None => {
                info!(server = %client.name, "the peer server is no longer in the paired server list, stopping forwarding and probes");
                peers.remove(&client.name);
                return;
            }
        }
        tokio::time::sleep(client.next_probe_wait(interval)).await;
        let result = client.probe(timeout).await;
        client.settle(result);
    }
}

/// 监听连接关闭，把断开原因写进状态。只持有弱引用，不会让连接或客户端多活。
fn watch_close(client: Weak<PeerClient>, conn: &Connection) {
    let closed = conn.weak_handle().closed();
    let conn_id = conn.stable_id();
    tokio::spawn(async move {
        let Some(closed) = closed.await else {
            return;
        };
        let Some(client) = client.upgrade() else {
            return;
        };
        // 本机断开的连接，断开的地方已经写了状态或换上了新连接。
        if matches!(closed.reason, ConnectionError::LocallyClosed) {
            return;
        }
        let err = from_connection_error(&closed.reason);
        client.note_broken(conn_id, err);
    });
}

/// 在指定的连接上做一次 ping 往返，不经过 [`PeerClient::connection`]。
async fn ping_on(conn: &Connection) -> Result<(), TunnelError> {
    let (mut send, mut recv) = conn
        .open_bi()
        .await
        .map_err(|err| from_connection_error(&err))?;
    send.write_all(&[Request::Ping as u8])
        .await
        .map_err(|err| from_write_error(&err))?;
    send.finish().ok();
    let mut byte = [0u8; 1];
    recv.read_exact(&mut byte)
        .await
        .map_err(|err| from_read_exact_error(&err))?;
    match Status::from_byte(byte[0]) {
        Some(Status::Ok) => Ok(()),
        Some(_) => Err(TunnelError::protocol(
            "the peer does not recognize the ping request",
        )),
        None => Err(TunnelError::protocol(format!(
            "unknown status byte {}",
            byte[0]
        ))),
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
