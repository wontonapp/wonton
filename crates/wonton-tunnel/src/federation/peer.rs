//! 到一台对方服务器的出站连接：所有流复用一条连接，断开后下一次使用时按退避重连，定时探活。
//!
//! 连接、退避、探活、按连接确认授权都是隧道客户端（[`TunnelClient`]，ALPN `wonton/federation/0`）的；
//! 这里只按 Matrix 服务器名管住每台的客户端和状态。隧道客户端报“被拒绝”就当作对方把本机移出了名单，
//! 名单恢复后下一次探活重新连上。

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::{
        Arc, Mutex, MutexGuard, PoisonError,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, SystemTime},
};

use iroh::{Endpoint, EndpointAddr, EndpointId, endpoint::RecvStream};
use tokio::{sync::watch, task::AbortHandle};
use tracing::info;

use crate::{
    TunnelClient, TunnelError, TunnelStatus, client::TunnelStream, federation::FEDERATION_ALPN,
    wire::Request,
};

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

/// 出站侧按服务器名跟踪的对方服务器：每台一个 [`PeerClient`]、一个探活任务和一个状态翻译任务。
#[derive(Debug, Default)]
pub(crate) struct PeerTable {
    peers: Mutex<HashMap<String, Tracked>>,
}

#[derive(Debug)]
struct Tracked {
    client: Arc<PeerClient>,
    ping: AbortHandle,
    status: AbortHandle,
}

impl PeerTable {
    /// 取这台对方服务器的客户端并更新它的地址，没有就建一个并开始定时探活。endpoint ID 换了（重新配对）
    /// 时整个换掉：旧连接和任务都作废。
    pub(crate) fn get_or_create(
        self: &Arc<Self>,
        name: &str,
        id: EndpointId,
        addr: EndpointAddr,
        ctx: &Arc<super::outbound::ForwarderCtx>,
    ) -> Arc<PeerClient> {
        let mut peers = lock(&self.peers);
        if let Some(tracked) = peers.get(name)
            && tracked.client.id() == id
        {
            tracked.client.set_addr(addr);
            return tracked.client.clone();
        }
        let client = Arc::new(PeerClient::new(
            name.to_owned(),
            ctx.endpoint.clone(),
            id,
            addr,
            ctx.status.clone(),
        ));
        // 先起任务、先占位：探活任务拆掉自己（remove）时一定找得到条目。
        let ping =
            tokio::spawn(ping_loop(client.clone(), self.clone(), ctx.clone())).abort_handle();
        let status = tokio::spawn(bridge_status(client.clone())).abort_handle();
        let replaced = peers.insert(
            name.to_owned(),
            Tracked {
                client: client.clone(),
                ping,
                status,
            },
        );
        if let Some(old) = replaced {
            old.ping.abort();
            old.status.abort();
            old.client.hang_up();
        }
        client
    }

    /// 拆掉一台对方服务器：停任务、断连接、状态条目删掉。
    pub(crate) fn remove(self: &Arc<Self>, name: &str) {
        let Some(tracked) = lock(&self.peers).remove(name) else {
            return;
        };
        tracked.client.mark_removed();
        tracked.ping.abort();
        tracked.status.abort();
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

    /// 停掉所有任务并断开连接。转发器停止时调用。
    pub(crate) fn stop_all(&self) {
        for (_, tracked) in lock(&self.peers).drain() {
            tracked.ping.abort();
            tracked.status.abort();
            tracked.client.hang_up();
        }
    }
}

/// 到一台对方服务器的连接。引用共享同一个隧道客户端、退避和状态。
#[derive(Debug)]
pub(crate) struct PeerClient {
    /// 对方的 Matrix 服务器名，也是状态表里的键。服务器名和 endpoint ID 的绑定只在配对时定（ADR-0050）。
    name: String,
    tunnel: TunnelClient,
    status: watch::Sender<PeerStates>,
    /// 已从配对名单里拆掉：晚到的状态不再把删掉的条目写回去。
    removed: AtomicBool,
}

impl PeerClient {
    fn new(
        name: String,
        endpoint: Endpoint,
        id: EndpointId,
        mut addr: EndpointAddr,
        status: watch::Sender<PeerStates>,
    ) -> Self {
        addr.id = id;
        Self {
            name,
            tunnel: TunnelClient::with_alpn(endpoint, addr, FEDERATION_ALPN),
            status,
            removed: AtomicBool::new(false),
        }
    }

    fn id(&self) -> EndpointId {
        self.tunnel.server_id()
    }

    /// 更新对方的地址（来自最新的配对信息）。endpoint ID 变了由 [`PeerTable`] 换客户端。
    fn set_addr(&self, mut addr: EndpointAddr) {
        addr.id = self.id();
        self.tunnel.set_server_addr(addr);
    }

    /// 开一条转发流：复用现有连接，或建一条新连接（受退避约束）。还在退避时直接失败，和换之前一样，
    /// 不等满请求等连接的时限。
    pub(crate) async fn open_forward(&self) -> Result<TunnelStream, TunnelError> {
        if !self.tunnel.has_connection() && !self.tunnel.backoff_left().is_zero() {
            return Err(TunnelError::transport(
                "waiting for the backoff before reconnecting",
            ));
        }
        self.tunnel.open(Request::Forward).await
    }

    /// 读对方在流上回的状态字节；回了状态说明这条连接确实是通的。
    pub(crate) async fn read_status(
        &self,
        recv: &mut RecvStream,
        conn_id: usize,
    ) -> Result<(), TunnelError> {
        self.tunnel.read_status(recv, conn_id).await
    }

    /// 限时确认到对方的连接真的通：有连接时在它上面做一次 ping 往返，没回应就换连接。没有连接时建一条
    /// 再探；还在退避时不试，返回 `Ok(None)`，等下个周期。建连有自己的上限，`timeout` 只管探活这一次往返。
    pub(crate) async fn probe(&self, timeout: Duration) -> Result<Option<Duration>, TunnelError> {
        if !self.tunnel.has_connection() {
            if !self.tunnel.backoff_left().is_zero() {
                return Ok(None);
            }
            // 被对方拒绝过的建连不会自己重拨，等 `retry_now`：名单恢复后下一次探活就能连上。
            if matches!(
                self.tunnel.status(),
                TunnelStatus::Disconnected(TunnelError::Unauthorized)
            ) {
                self.tunnel.retry_now();
            }
        }
        self.tunnel.probe(timeout).await.map(Some)
    }

    /// 下一次探活要等多久：有连接按周期；没有连接时按退避，没在退避就立刻试。
    pub(crate) fn next_probe_wait(&self, interval: Duration) -> Duration {
        if self.tunnel.has_connection() {
            return interval;
        }
        self.tunnel.backoff_left()
    }

    /// 探活结果落状态。`Ok(None)` 是这次没探（还在退避），状态不动；连上由状态翻译任务写。
    pub(crate) fn settle(&self, result: Result<Option<Duration>, TunnelError>) {
        match result {
            Ok(_) => {}
            Err(TunnelError::Unauthorized) => self.set_status(PeerStatus::RemovedByPeer),
            Err(err) => self.set_offline(err),
        }
    }

    /// 关掉连接（本机主动关闭，关闭事件不写状态）。
    fn hang_up(&self) {
        self.tunnel.close();
    }

    fn mark_removed(&self) {
        self.removed.store(true, Ordering::Relaxed);
    }

    /// 名字已从配对名单里删除，状态条目随之消失。
    fn remove_state(&self) {
        self.status
            .send_if_modified(|states| states.remove(&self.name).is_some());
    }

    fn set_offline(&self, err: TunnelError) {
        if self.removed.load(Ordering::Relaxed) {
            return;
        }
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
        if self.removed.load(Ordering::Relaxed) {
            return;
        }
        self.status.send_if_modified(|states| {
            if states.get(&self.name) == Some(&status) {
                return false;
            }
            states.insert(self.name.clone(), status);
            true
        });
    }

    /// 连接被对方（或传输）关闭时写状态。隧道客户端报“被拒绝”就是对方名单里没有本机。
    fn note_broken(&self, err: TunnelError) {
        if err == TunnelError::Unauthorized {
            info!(server = %self.name, "the peer server rejected this server");
            self.set_status(PeerStatus::RemovedByPeer);
        } else {
            info!(server = %self.name, "federation connection lost: {err}");
            self.set_offline(err);
        }
    }
}

/// 把隧道客户端的状态翻成这台对方服务器的状态。
async fn bridge_status(client: Arc<PeerClient>) {
    let mut status = client.tunnel.watch_status();
    loop {
        match status.borrow_and_update().clone() {
            TunnelStatus::Connected => client.set_status(PeerStatus::Connected),
            TunnelStatus::Disconnected(err) => client.note_broken(err),
            // 正在建连、还没连过和已关闭都不改变状态：前两者不把刚记下的原因冲掉，关闭后条目马上会被删。
            TunnelStatus::Idle | TunnelStatus::Connecting | TunnelStatus::Closed => {}
        }
        if status.changed().await.is_err() {
            return;
        }
    }
}

/// 定时探活一台对方服务器。名字从查找函数里消失（对方已从配对名单里删除）时，把它整个拆掉；endpoint ID
/// 换了（重新配对）时换上的客户端有自己的探活任务，这个退出。
async fn ping_loop(
    client: Arc<PeerClient>,
    peers: Arc<PeerTable>,
    ctx: Arc<super::outbound::ForwarderCtx>,
) {
    loop {
        let Some((id, addr)) = (ctx.lookup)(&client.name) else {
            info!(
                server = %client.name,
                "the peer server is no longer in the paired server list, stopping forwarding and probes"
            );
            peers.remove(&client.name);
            return;
        };
        if !Arc::ptr_eq(&peers.get_or_create(&client.name, id, addr, &ctx), &client) {
            return;
        }
        tokio::time::sleep(client.next_probe_wait(ctx.ping_interval)).await;
        let result = client.probe(ctx.ping_timeout).await;
        client.settle(result);
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
