//! 到一台对方服务器的出站连接：[`TunnelClient`] 按 [`FEDERATION_ALPN`] 连过去，所有流复用一条连接，
//! 断开后下一次使用时自动重连，定时探活。
//!
//! `TunnelClient` 只知道自己这条连接的 [`TunnelStatus`]；这里把它翻成按 Matrix 服务器名索引的
//! [`PeerStatus`](crate::federation::PeerStatus)（[`status_loop`]），共享连接、退避、判死都由
//! `TunnelClient` 负责（ADR-0032）。

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    time::{Duration, SystemTime},
};

use iroh::{Endpoint, EndpointAddr, EndpointId};
use tokio::{sync::watch, task::AbortHandle};
use tracing::info;

use crate::{
    client::{TunnelClient, TunnelStatus, TunnelStream},
    error::TunnelError,
    federation::{FEDERATION_ALPN, ServerLookup},
    wire::Request,
};

/// 一台对方服务器的连接状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PeerStatus {
    /// 最近一次探活或数据往返成功。
    Connected,
    /// 连不上对方：`since` 是变成离线的时刻，`last_error` 是最近一次失败的原因。下一次使用或下一次
    /// 探活时自动重连。
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

/// 出站侧按服务器名跟踪的对方服务器：每台一个 [`PeerClient`] 和探活、状态翻译两个任务。
#[derive(Debug)]
pub(crate) struct PeerTable {
    peers: Mutex<HashMap<String, Tracked>>,
    status: watch::Sender<PeerStates>,
}

#[derive(Debug)]
struct Tracked {
    /// 建这台客户端时用的对方 endpoint ID。换 ID（重新配对）就整台换新；不放在 [`PeerClient`] 里，
    /// 探活任务刷新地址时不会动它。
    id: EndpointId,
    client: Arc<PeerClient>,
    ping: AbortHandle,
    status: AbortHandle,
}

impl PeerTable {
    pub(crate) fn new(status: watch::Sender<PeerStates>) -> Self {
        Self {
            peers: Mutex::new(HashMap::new()),
            status,
        }
    }

    /// 取这台对方服务器的客户端，没有就建一个，并开始定时探活。
    pub(crate) fn get_or_create(
        self: &Arc<Self>,
        name: &str,
        id: EndpointId,
        addr: EndpointAddr,
        ctx: &super::outbound::ForwarderCtx,
    ) -> Arc<PeerClient> {
        let mut peers = lock(&self.peers);
        if let Some(tracked) = peers.get(name)
            && tracked.id == id
        {
            return tracked.client.clone();
        }
        // 新的一台，或者 endpoint ID 换了（重新配对）：旧的整个拆掉，连到旧 endpoint ID 的连接作废
        // （ADR-0050：服务器名和 endpoint ID 的绑定换新，旧的就不再可信）。
        let replaced = peers.remove(name);
        let client = PeerClient::new(name.to_owned(), addr.clone(), ctx.endpoint.clone());
        let ping = tokio::spawn(ping_loop(
            client.clone(),
            self.clone(),
            ctx.lookup.clone(),
            ctx.ping_interval,
            ctx.ping_timeout,
        ))
        .abort_handle();
        let status = tokio::spawn(status_loop(
            name.to_owned(),
            client.watch_status(),
            self.status.clone(),
        ))
        .abort_handle();
        peers.insert(
            name.to_owned(),
            Tracked {
                id,
                client: client.clone(),
                ping,
                status,
            },
        );
        drop(peers);
        if let Some(replaced) = replaced {
            info!(server = %name, "the peer server re-paired with a new endpoint ID, dropping the old connection");
            replaced.ping.abort();
            replaced.status.abort();
            replaced.client.close();
        }
        client
    }

    /// 拆掉一台对方服务器：停任务、断连接、状态条目删掉。
    pub(crate) fn remove(self: &Arc<Self>, name: &str) {
        let Some(tracked) = lock(&self.peers).remove(name) else {
            return;
        };
        tracked.ping.abort();
        tracked.status.abort();
        tracked.client.close();
        // 名字已从配对名单里删除，状态条目随之消失。
        self.status
            .send_if_modified(|states| states.remove(name).is_some());
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

    /// 停掉所有任务并断开连接。转发器停止时调用；状态条目留给正在订阅的嵌入方看最后一份。
    pub(crate) fn stop_all(&self) {
        for (_, tracked) in lock(&self.peers).drain() {
            tracked.ping.abort();
            tracked.status.abort();
            tracked.client.close();
        }
    }
}

/// 到一台对方服务器的连接。引用共享同一条连接、退避和状态。
#[derive(Debug)]
pub(crate) struct PeerClient {
    /// 对方的 Matrix 服务器名，也是状态表里的键。服务器名和 endpoint ID 的绑定只在配对时定（ADR-0050）。
    name: String,
    client: TunnelClient,
}

impl PeerClient {
    fn new(name: String, addr: EndpointAddr, endpoint: Endpoint) -> Arc<Self> {
        Arc::new(Self {
            name,
            client: TunnelClient::with_alpn(endpoint, addr, FEDERATION_ALPN),
        })
    }

    /// 用最新的查找结果更新对方的地址；endpoint ID 变了由 [`PeerTable::get_or_create`] 整台换新。
    pub(crate) fn set_target(&self, addr: EndpointAddr) {
        self.client.set_server_addr(addr);
    }

    /// 订阅这条连接的状态，[`status_loop`] 翻译成这台服务器的 [`PeerStatus`]。
    pub(crate) fn watch_status(&self) -> watch::Receiver<TunnelStatus> {
        self.client.watch_status()
    }

    /// 开一条转发流：复用现有连接，或等下一次建连（受退避约束）。
    pub(crate) async fn open_forward(&self) -> Result<TunnelStream, TunnelError> {
        self.client.open(Request::Forward).await
    }

    /// 读对方在这条流上回的状态字节。回状态本身就确认了连接，状态由 `TunnelClient` 记下。
    pub(crate) async fn read_status(&self, stream: &mut TunnelStream) -> Result<(), TunnelError> {
        self.client
            .read_status(&mut stream.recv, stream.conn_id)
            .await
    }

    /// 限时确认到对方的连接真的通：有连接时在它上面做一次 ping 往返，没有回应、窗口里也没有别的
    /// 证据就换连接；没有连接时建一条再探。`timeout` 只管探活这一次往返。
    pub(crate) async fn probe(&self, timeout: Duration) -> Result<Duration, TunnelError> {
        self.client.probe(timeout).await
    }

    fn close(&self) {
        self.client.close();
    }
}

/// 定时探活一台对方服务器。名字从查找函数里消失（对方已从配对名单里删除）时，把它整个拆掉。
/// 探活的结果由 `TunnelClient` 写进连接状态，[`status_loop`] 翻译成 [`PeerStatus`]，这里不用管。
async fn ping_loop(
    client: Arc<PeerClient>,
    peers: Arc<PeerTable>,
    lookup: ServerLookup,
    interval: Duration,
    timeout: Duration,
) {
    let mut first = true;
    loop {
        match lookup(&client.name) {
            Some((_id, addr)) => client.set_target(addr),
            None => {
                info!(server = %client.name, "the peer server is no longer in the paired server list, stopping forwarding and probes");
                peers.remove(&client.name);
                return;
            }
        }
        // 配对成立马上探第一次，之后按周期。
        tokio::time::sleep(if first { Duration::ZERO } else { interval }).await;
        first = false;
        let _ = client.probe(timeout).await;
    }
}

/// 把 [`TunnelClient`] 的连接状态翻成这台服务器的 [`PeerStatus`] 写进共享状态表。
///
/// `Idle` 和 `Connecting` 不写：表里没有这台 = 还没有探活结果，嵌入方按“正在确认”处理（ADR-0050）；
/// `Closed` 是本机拆掉这台（客户端已换或已停），条目由拆的地方（[`PeerTable`]）删。
async fn status_loop(
    name: String,
    mut status: watch::Receiver<TunnelStatus>,
    states: watch::Sender<PeerStates>,
) {
    loop {
        apply_status(&name, &status.borrow_and_update(), &states);
        if status.changed().await.is_err() {
            return;
        }
    }
}

fn apply_status(name: &str, status: &TunnelStatus, states: &watch::Sender<PeerStates>) {
    let next = match status {
        TunnelStatus::Connected => Some(PeerStatus::Connected),
        TunnelStatus::Disconnected(TunnelError::Unauthorized) => Some(PeerStatus::RemovedByPeer),
        TunnelStatus::Disconnected(err) => Some(offline_status(states, name, err)),
        TunnelStatus::Idle | TunnelStatus::Connecting | TunnelStatus::Closed => None,
    };
    if let Some(next) = next {
        states.send_if_modified(|map| {
            if map.get(name) == Some(&next) {
                return false;
            }
            map.insert(name.to_owned(), next);
            true
        });
    }
}

/// 连不上对方：`since` 保持第一次变成离线的时刻，错误刷新成最近一次的。
fn offline_status(states: &watch::Sender<PeerStates>, name: &str, err: &TunnelError) -> PeerStatus {
    let since = match states.borrow().get(name) {
        Some(PeerStatus::Offline { since, .. }) => *since,
        _ => SystemTime::now(),
    };
    PeerStatus::Offline {
        since,
        last_error: err.clone(),
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
