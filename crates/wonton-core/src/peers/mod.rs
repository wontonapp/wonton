//! 服务器配对（ADR-0050）：主人的码长期有效，对方扫码或粘贴后发来请求，出码方的主人同意，两台服务器的
//! Matrix 联邦走 iroh。
//!
//! - 配对记录存 `core/peers.json`，服务器名和对方 endpoint ID 的绑定只在配对时定下、以后不变。
//! - 出码：每个主人只有一张码（`code::PeerPairToken` 落盘，同其他密钥的权限），一直有效到重置。码是
//!   `wonton://peer?…`，格式由核心定，壳只显示和分享。
//! - 收请求：验令牌、按限频收下（落盘），在“Wonton”私聊里提醒主人，等主人同意或拒绝。
//! - 扫码：主人的设备把码交给核心（`/_wonton/api/peers`），核心经隧道的 `wonton/peer/1` 连出码方；
//!   成功后记成“等对方同意”，启动时和之后每十分钟问一次结果，对方推送也收。
//! - 同意：两边记成已配对；拒绝：记进“已拒绝”，同一台服务器再发直接回“对方拒绝了”，主人能撤销。
//! - 两边互相发了请求：直接成立，两边都不用再点。
//! - 删配对：记录删掉、联邦名单撤掉、通知对方一次（尽力而为）；自己发出的请求用同一路径取消。
//! - 对方删了配对：对方收到通知后（或握手被对方拒绝时）记录留着、标成“对方删了”，联邦名单同样撤掉；
//!   主人在 App 里移除这一行才真删记录，重新配对时这条记录被新配对替换。
//!
//! 联邦转发用的接口：[`Peers::server_allowlist`]（入站名单）、[`Peers::lookup`]（服务器名 → endpoint ID，
//! 核心层的“一个服务器名只会转发到配对时绑定的 endpoint ID”）、[`Peers::paired_names`]（已配对的服务器名，
//! 给 Tuwunel 配置的订阅）。

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures_util::future::BoxFuture;
use iroh::{EndpointAddr, EndpointId, RelayUrl};
use serde::{Deserialize, Serialize};
use tokio::sync::watch;
use wonton_protocol::{
    PeerCode, PeerRequestRow, PeerRow, PeerStatus, PeersView, RejectedPeerRow, SecretString,
};
use wonton_tunnel::Allowlist;
use wonton_tunnel::federation::{PeerStates, PeerStatus as FederationStatus};
use wonton_tunnel::peer_pair::{PeerIdentity, PeerPairingCode};
use wonton_tunnel::relays::canonical_url;

use crate::plugin::languages::Languages;
use crate::tunnel::Tunnel;

mod code;
pub mod pairing;
#[cfg(test)]
mod peers_tests;
mod scanning;

/// 待处理的请求最多几条（ADR-0050：超出回“对方暂时收不了请求”）。
const MAX_PENDING: usize = 20;

/// 一小时最多收几条新请求（ADR-0050）。
const MAX_PER_HOUR: usize = 10;

/// 一小时的毫秒数。
const HOUR_MS: u64 = 60 * 60 * 1000;

/// 配对的进展。等对方同意和对方拒绝的行由 [`Phase::AwaitingPeer`] / [`Phase::Declined`] 表示。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// 已配对：联邦名单里有它。
    Paired,
    /// 我扫了对方，对方的主人还没处理。
    AwaitingPeer,
    /// 对方拒绝了我的请求。
    Declined,
    /// 对方删了这条配对。记录留着给主人看，等主人在 App 里移除；重新配对时被新记录替换。
    RemovedByPeer,
}

/// 一条配对记录。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerRecord {
    pub server_name: String,
    pub endpoint_id: EndpointId,
    /// 能被回连的地址：中继在前，直连跟上。配对时从码或扫码请求里记下。
    #[serde(default)]
    pub relays: Vec<String>,
    #[serde(default)]
    pub addrs: Vec<SocketAddr>,
    pub owner_user_id: String,
    pub owner_display_name: String,
    /// 配对成立的时间（Unix 毫秒）。
    pub paired_at_ms: u64,
    pub phase: Phase,
    /// 最近一次连上对方的时刻（Unix 毫秒）。转发组件的状态变成“已连上”时记下，离线时保留。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_connected_ms: Option<u64>,
}

impl PeerRecord {
    /// 回连这一端用的地址。
    pub(crate) fn endpoint_addr(&self) -> EndpointAddr {
        endpoint_addr(self.endpoint_id, &self.relays, &self.addrs)
    }
}

/// 别的服务器发来、还没处理的一条请求。除了 `id`，都是对方自报的。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerRequest {
    /// 核心给的请求 ID。请求按它认，不按服务器名：服务器名是自报的，两条请求可能报同一个。
    pub id: String,
    /// QUIC 握手认证的对方 endpoint ID。
    pub endpoint_id: EndpointId,
    pub server_name: String,
    pub owner_user_id: String,
    pub owner_display_name: String,
    #[serde(default)]
    pub relays: Vec<String>,
    #[serde(default)]
    pub addrs: Vec<SocketAddr>,
    /// 收到的时刻（Unix 毫秒）；同一台服务器再发时是最后一次的时刻。
    pub received_at_ms: u64,
}

impl PeerRequest {
    fn endpoint_addr(&self) -> EndpointAddr {
        endpoint_addr(self.endpoint_id, &self.relays, &self.addrs)
    }
}

/// 主人拒绝过的一条请求。同一台服务器（按 endpoint ID）再发来的请求直接回“对方拒绝了”，撤销后照常收下。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RejectedRequest {
    pub id: String,
    pub endpoint_id: EndpointId,
    pub server_name: String,
    pub owner_user_id: String,
    pub owner_display_name: String,
    /// 拒绝的时刻（Unix 毫秒）。
    pub rejected_at_ms: u64,
}

fn endpoint_addr(id: EndpointId, relays: &[String], addrs: &[SocketAddr]) -> EndpointAddr {
    let mut addr = EndpointAddr::new(id);
    for relay in relays {
        if let Ok(relay) = relay.parse::<RelayUrl>() {
            addr = addr.with_relay_url(canonical_url(relay));
        }
    }
    for ip in addrs {
        addr = addr.with_ip_addr(*ip);
    }
    addr
}

/// 自己这边的身份，配对时发给对方。主人的显示名不在这里：它在每次握手时现取（主人会改资料）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnIdentity {
    pub server_name: String,
    pub owner_user_id: String,
}

impl OwnIdentity {
    pub(crate) fn to_wire(&self, tunnel: &Tunnel, owner_display_name: String) -> PeerIdentity {
        PeerIdentity {
            server_name: self.server_name.clone(),
            owner_user_id: self.owner_user_id.clone(),
            owner_display_name,
            relays: tunnel
                .relays()
                .iter()
                .map(|relay| relay.url.to_string())
                .collect(),
            addrs: tunnel.direct_addrs(),
        }
    }
}

/// 在“Wonton”私聊里提醒主人（生产里是 `matrix::NoticeRoom`）。发不出去只记日志，不影响配对。
pub trait OwnerNotices: Send + Sync + 'static {
    /// 发一条消息，内容是 Matrix 的消息体（`m.text`）。
    fn send(&self, content: serde_json::Value) -> BoxFuture<'_, Result<(), String>>;
}

impl OwnerNotices for crate::matrix::NoticeRoom {
    fn send(&self, content: serde_json::Value) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async move {
            self.send(&content)
                .await
                .map(|_| ())
                .map_err(|e| e.to_string())
        })
    }
}

/// 记录写入失败。
#[derive(Debug, thiserror::Error)]
pub enum PeerStoreError {
    #[error("读写配对记录 {path} 失败：{source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("配对记录 {path} 已损坏：{message}")]
    Corrupt { path: PathBuf, message: String },
}

/// 服务器配对：记录、出码、扫码、请求、删配对，以及联邦名单。
pub struct Peers {
    path: PathBuf,
    own: OwnIdentity,
    tunnel: Tunnel,
    /// 出码方的联邦入站名单：已配对的 endpoint ID。
    server_allowlist: Allowlist,
    /// 已配对的服务器名，配对和删除时即时更新。给 Tuwunel 配置（名单和 /etc/hosts）订阅。
    names: watch::Sender<BTreeSet<String>>,
    /// 联邦转发组件的状态。还没接转发组件时，已配对的行按“正在确认”处理。
    forwarder: Mutex<Option<watch::Receiver<PeerStates>>>,
    /// 主人在“Wonton”私聊里的提醒。
    notices: Option<Arc<dyn OwnerNotices>>,
    /// 主人的显示名，每次握手现取：主人改了资料，下次配对用的就是新名字。
    owner_display_name: Arc<dyn Fn() -> String + Send + Sync>,
    /// 主人的语言，说明和提醒按它写（ADR-0043）。
    languages: Arc<dyn Fn() -> Languages + Send + Sync>,
    /// 测试用：每写一次盘推进一版，等状态变化用。
    #[cfg(test)]
    changes: watch::Sender<u64>,
    inner: Mutex<Inner>,
}

#[derive(Debug, Default)]
struct Inner {
    /// 服务器名 → 记录。
    peers: BTreeMap<String, PeerRecord>,
    /// 别的服务器发来的请求，一台服务器最多一条。
    requests: Vec<PeerRequest>,
    /// 主人拒绝过的请求。
    rejected: Vec<RejectedRequest>,
    /// 本机的请求令牌。和记录一起写盘，权限相同。
    token: Option<code::PeerPairToken>,
    /// 收下新请求的时刻，算这一小时的条数。
    received: Vec<u64>,
}

impl fmt::Debug for Peers {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Peers")
            .field("own", &self.own)
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or_default()
}

impl Peers {
    /// 读取记录；文件不在时从空表开始，顺手发一张码。
    ///
    /// 上次没处理完的请求留下来（重启不再丢），还在等对方同意的记录重启后接着问结果。
    pub fn open(
        path: impl Into<PathBuf>,
        own: OwnIdentity,
        tunnel: Tunnel,
        notices: Option<Arc<dyn OwnerNotices>>,
        owner_display_name: Arc<dyn Fn() -> String + Send + Sync>,
        languages: Arc<dyn Fn() -> Languages + Send + Sync>,
    ) -> Result<Arc<Self>, PeerStoreError> {
        let path = path.into();
        let mut stored = read_store(&path)?;
        stored
            .received
            .retain(|at| at.saturating_add(HOUR_MS) > now_ms());
        let created_token = stored.token.is_none();
        if created_token {
            stored.token = Some(code::issue());
        }
        let paired: BTreeSet<String> = stored
            .peers
            .values()
            .filter(|record| record.phase == Phase::Paired)
            .map(|record| record.server_name.clone())
            .collect();
        let server_allowlist = Allowlist::new(
            stored
                .peers
                .values()
                .filter(|record| record.phase == Phase::Paired)
                .map(|record| record.endpoint_id),
        );
        let waiting: Vec<String> = stored
            .peers
            .values()
            .filter(|record| record.phase == Phase::AwaitingPeer)
            .map(|record| record.server_name.clone())
            .collect();
        let (names, _) = watch::channel(paired);
        let peers = Arc::new(Self {
            inner: Mutex::new(Inner {
                peers: stored.peers,
                requests: stored.requests,
                rejected: stored.rejected,
                token: stored.token,
                received: stored.received,
            }),
            path,
            own,
            tunnel,
            names,
            server_allowlist,
            forwarder: Mutex::new(None),
            notices,
            owner_display_name,
            languages,
            #[cfg(test)]
            changes: watch::channel(0).0,
        });
        if created_token {
            let inner = peers.lock();
            if let Err(e) = peers.save(&inner) {
                tracing::error!(error = %e, "新配的码写不进去");
            }
        }
        for name in waiting {
            peers.spawn_poll(&name);
        }
        Ok(peers)
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().expect("配对表锁中毒")
    }

    /// 整份重写记录：先写临时文件再改名，避免写到一半断电留下损坏的记录（照 `devices.json` 的做法）。
    /// 调用方持着 `self.lock()`，这把锁同时串行化写盘。
    fn save(&self, inner: &Inner) -> Result<(), PeerStoreError> {
        let io = |source| PeerStoreError::Io {
            path: self.path.clone(),
            source,
        };
        let stored = Stored {
            peers: inner.peers.clone(),
            requests: inner.requests.clone(),
            rejected: inner.rejected.clone(),
            token: inner.token.clone(),
            received: inner.received.clone(),
        };
        let text = serde_json::to_string_pretty(&stored).map_err(|e| PeerStoreError::Corrupt {
            path: self.path.clone(),
            message: e.to_string(),
        })?;
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(io)?;
        }
        let tmp = self.path.with_extension("json.tmp");
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)
            .map_err(io)?;
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        file.write_all(text.as_bytes()).map_err(io)?;
        file.sync_all().map_err(io)?;
        std::fs::rename(&tmp, &self.path).map_err(io)?;
        #[cfg(test)]
        self.changes.send_modify(|version| *version += 1);
        Ok(())
    }

    /// 测试用：等记录或名单变化的版本号，比隔一会儿问一次稳。
    #[cfg(test)]
    pub(crate) fn changes(&self) -> watch::Receiver<u64> {
        self.changes.subscribe()
    }

    /// 测试用：隧道入口（endpoint、设备登记表和设备配对处理方都在里面）。
    #[cfg(test)]
    pub(crate) fn tunnel(&self) -> &Tunnel {
        &self.tunnel
    }

    /// 出码方的联邦入站名单，挂在联邦协议上：名单里的 endpoint ID 才能进联邦。
    pub fn server_allowlist(&self) -> &Allowlist {
        &self.server_allowlist
    }

    /// 已配对的服务器名，配对和删除时即时更新。给 Tuwunel 的配置（联邦名单和 `/etc/hosts`）订阅；
    /// 名单变化后由订阅方改 Tuwunel 的配置并发 `SIGUSR1`（fed-tuwunel-config 的任务）。
    pub fn paired_names(&self) -> watch::Receiver<BTreeSet<String>> {
        self.names.subscribe()
    }

    /// 联邦出站的查找函数：Matrix 服务器名 → 配对时记下的对方 endpoint ID 和地址。只认已配对的记录；
    /// 一个服务器名只会转发到配对时绑定的 endpoint ID（ADR-0050 的“代价与风险”）。
    pub fn lookup(&self, server_name: &str) -> Option<(EndpointId, EndpointAddr)> {
        let peers = self.lock();
        let record = peers.peers.get(server_name)?;
        (record.phase == Phase::Paired).then(|| (record.endpoint_id, record.endpoint_addr()))
    }

    /// 这次的配对握手里要发给对方的身份。主人的显示名现取，主人改资料后不用重启核心。
    pub(crate) fn own_identity(&self) -> PeerIdentity {
        self.own.to_wire(&self.tunnel, (self.owner_display_name)())
    }

    /// 本机的码：令牌一直是这一张，中继地址按当时的取。
    pub fn issue_code(&self) -> PeerCode {
        let inner = self.lock();
        let token = match inner.token.clone() {
            Some(token) => token,
            None => {
                let token = code::issue();
                let mut inner = inner;
                inner.token = Some(token.clone());
                if let Err(e) = self.save(&inner) {
                    tracing::error!(error = %e, "新配的码写不进去");
                }
                token
            }
        };
        self.code_uri(&token)
    }

    /// 重置码：换一个新令牌，旧码作废，用旧码发来的待处理请求一并清掉。
    pub fn reset_code(&self) -> PeerCode {
        let token = code::issue();
        {
            let mut inner = self.lock();
            inner.token = Some(token.clone());
            inner.requests.clear();
            if let Err(e) = self.save(&inner) {
                tracing::error!(error = %e, "重置的码写不进去");
            }
        }
        tracing::info!("主人重置了服务器配对码");
        self.code_uri(&token)
    }

    fn code_uri(&self, token: &code::PeerPairToken) -> PeerCode {
        PeerCode {
            code: SecretString::new(
                PeerPairingCode {
                    server_id: self.tunnel.endpoint.id(),
                    relay_urls: self
                        .tunnel
                        .relays()
                        .iter()
                        .map(|relay| relay.url.clone())
                        .collect(),
                    server_name: self.own.server_name.clone(),
                    token: token.clone(),
                }
                .to_uri(),
            ),
        }
    }

    /// 接上联邦转发组件：已配对行的状态和“最近连上”的时刻从这里来。
    ///
    /// 状态变化跟着记：连上时把时刻写进记录（持久化，重启后还在），对方拒绝本机时把记录标成“对方删了”。
    pub fn attach_forwarder(self: &Arc<Self>, states: watch::Receiver<PeerStates>) {
        *self.forwarder.lock().expect("转发器状态锁中毒") = Some(states.clone());
        let peers = Arc::downgrade(self);
        tokio::spawn(async move {
            let mut states = states;
            // 订阅方的第一份值就是当前状态：接上之前已经连上的也要记一次。
            while states.changed().await.is_ok() {
                for (name, status) in states.borrow_and_update().clone() {
                    let Some(peers) = peers.upgrade() else {
                        return;
                    };
                    match status {
                        FederationStatus::Connected => peers.note_connected(&name),
                        FederationStatus::RemovedByPeer => peers.note_removed_by_peer(&name),
                        FederationStatus::Offline { .. } => {}
                    }
                }
            }
        });
    }

    /// 转发组件报“已连上”：记下时刻。同一段连接上反复报（数据往返也确认连接）时，同一毫秒不重复写盘。
    fn note_connected(&self, server_name: &str) {
        let mut inner = self.lock();
        let Some(record) = inner.peers.get_mut(server_name) else {
            return;
        };
        let now = now_ms();
        if record.last_connected_ms == Some(now) {
            return;
        }
        record.last_connected_ms = Some(now);
        if let Err(e) = self.save(&inner) {
            tracing::error!(error = %e, "配对记录写不进去");
        }
    }

    /// 联邦握手被对方拒绝：和收到删除通知一样，记录留着、标成“对方删了”、名单撤掉。
    pub(super) fn note_removed_by_peer(&self, server_name: &str) {
        let endpoint = {
            let mut inner = self.lock();
            let Some(record) = inner.peers.get_mut(server_name) else {
                return;
            };
            if record.phase != Phase::Paired {
                return;
            }
            record.phase = Phase::RemovedByPeer;
            let endpoint = record.endpoint_id;
            if let Err(e) = self.save(&inner) {
                tracing::error!(error = %e, "配对记录写不进去");
            }
            endpoint
        };
        self.retract(server_name, endpoint);
        tracing::info!(server = %server_name, "对方服务器拒绝了本机，记录标成对方已断开");
    }

    /// 主人看到的服务器列表、发来的请求和拒绝过的那几条。
    pub fn view(&self, langs: &Languages) -> PeersView {
        let inner = self.lock();
        let peers = inner
            .peers
            .values()
            .map(|record| self.row(record, langs))
            .collect::<Vec<_>>();
        let requests = inner
            .requests
            .iter()
            .map(|request| PeerRequestRow {
                id: request.id.clone(),
                server_name: request.server_name.clone(),
                owner_user_id: request.owner_user_id.clone(),
                owner_display_name: request.owner_display_name.clone(),
                received_at_ms: request.received_at_ms,
                conflicts_with: conflict(&inner, request),
                detail: Some(endpoint_text(request.endpoint_id, langs)),
            })
            .collect::<Vec<_>>();
        let rejected = inner
            .rejected
            .iter()
            .map(|rejected| RejectedPeerRow {
                id: rejected.id.clone(),
                server_name: rejected.server_name.clone(),
                owner_user_id: rejected.owner_user_id.clone(),
                owner_display_name: rejected.owner_display_name.clone(),
                rejected_at_ms: rejected.rejected_at_ms,
                detail: Some(endpoint_text(rejected.endpoint_id, langs)),
            })
            .collect::<Vec<_>>();
        PeersView {
            peers,
            requests,
            rejected,
        }
    }

    fn row(&self, record: &PeerRecord, langs: &Languages) -> PeerRow {
        let status = match record.phase {
            Phase::AwaitingPeer => PeerStatus::AwaitingApproval,
            Phase::Declined => PeerStatus::Declined,
            Phase::RemovedByPeer => PeerStatus::RemovedByPeer,
            Phase::Paired => self.forwarded_status(&record.server_name),
        };
        let text = if record.phase == Phase::AwaitingPeer {
            endpoint_text(record.endpoint_id, langs)
        } else {
            self.detail(record, langs)
        };
        PeerRow {
            server_name: record.server_name.clone(),
            owner_user_id: record.owner_user_id.clone(),
            owner_display_name: record.owner_display_name.clone(),
            status,
            last_connected_ms: record.last_connected_ms,
            detail: Some(text),
        }
    }

    /// 已配对的服务器按转发组件的状态给 [`PeerStatus`]（ADR-0050 的“状态”）。转发组件还没有这台的结果
    /// （刚配对、第一次探活还没出结果）时不报离线，给 `Checking`（壳显示“正在确认”）。
    ///
    fn forwarded_status(&self, server_name: &str) -> PeerStatus {
        let guard = self.forwarder.lock().expect("转发器状态锁中毒");
        let Some(states) = guard.as_ref() else {
            return PeerStatus::Checking;
        };
        match states.borrow().get(server_name) {
            Some(FederationStatus::Connected) => PeerStatus::Connected,
            Some(FederationStatus::Offline { .. }) => PeerStatus::Offline,
            Some(FederationStatus::RemovedByPeer) => PeerStatus::RemovedByPeer,
            None => PeerStatus::Checking,
        }
    }

    /// “详情”：endpoint ID 前 8 位、最后一次错误（ADR-0050 的状态表）。
    fn detail(&self, record: &PeerRecord, langs: &Languages) -> String {
        let id = short_endpoint(record.endpoint_id);
        match self.last_error(&record.server_name) {
            Some(error) => crate::l10n::text(
                langs,
                "peers-endpoint-detail",
                &[("id", id.into()), ("error", error.into())],
            ),
            None => endpoint_text(record.endpoint_id, langs),
        }
    }

    fn last_error(&self, server_name: &str) -> Option<String> {
        let guard = self.forwarder.lock().expect("转发器状态锁中毒");
        let states = guard.as_ref()?;
        match states.borrow().get(server_name)? {
            FederationStatus::Offline { last_error, .. } => Some(last_error.to_string()),
            _ => None,
        }
    }

    /// 在“Wonton”私聊里提醒主人有一条新请求，带跳到“其他服务器”的链接。
    fn notify(self: &Arc<Self>, request: &PeerRequest) {
        let Some(notices) = self.notices.clone() else {
            return;
        };
        let langs = (self.languages)();
        let who = if request.owner_display_name.trim().is_empty() {
            request.server_name.clone()
        } else {
            request.owner_display_name.clone()
        };
        let text = crate::l10n::text(
            &langs,
            "peers-new-request",
            &[
                ("owner", who.into()),
                ("server", request.server_name.as_str().into()),
            ],
        ) + "\n"
            + &wonton_protocol::AppLink::Peers.to_url();
        let server_name = request.server_name.clone();
        tokio::spawn(async move {
            let content = serde_json::json!({"msgtype": "m.text", "body": text});
            if let Err(e) = notices.send(content).await {
                tracing::info!(server = %server_name, error = %e, "配对提醒没有发进私聊");
            }
        });
    }
}

/// 自报的名字和一台已配对的服务器重名、endpoint 不同时，是那台的显示名（没有时是服务器名）。
fn conflict(inner: &Inner, request: &PeerRequest) -> Option<String> {
    let same_display = !request.owner_display_name.trim().is_empty();
    inner.peers.values().find_map(|record| {
        if record.endpoint_id == request.endpoint_id {
            return None;
        }
        let same = record.server_name == request.server_name
            || (same_display && record.owner_display_name == request.owner_display_name);
        same.then(|| {
            if record.owner_display_name.trim().is_empty() {
                record.server_name.clone()
            } else {
                record.owner_display_name.clone()
            }
        })
    })
}

/// endpoint ID 前 8 位。“endpoint”这个词由文案加（`peers-endpoint`、`peers-endpoint-detail`），这里不带。
fn short_endpoint(id: EndpointId) -> String {
    id.to_string()[..8].to_owned()
}

fn endpoint_text(id: EndpointId, langs: &Languages) -> String {
    crate::l10n::text(
        langs,
        "peers-endpoint",
        &[("id", short_endpoint(id).into())],
    )
}

/// `core/peers.json` 的形状。
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Stored {
    #[serde(default)]
    pub peers: BTreeMap<String, PeerRecord>,
    #[serde(default)]
    pub requests: Vec<PeerRequest>,
    #[serde(default)]
    pub rejected: Vec<RejectedRequest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<code::PeerPairToken>,
    /// 收下新请求的时刻，重启后照样算这一小时的条数。
    #[serde(default)]
    pub received: Vec<u64>,
}

fn read_store(path: &Path) -> Result<Stored, PeerStoreError> {
    match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str::<Stored>(&text).map_err(|e| PeerStoreError::Corrupt {
            path: path.to_owned(),
            message: e.to_string(),
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Stored::default()),
        Err(source) => Err(PeerStoreError::Io {
            path: path.to_owned(),
            source,
        }),
    }
}

/// 后台轮询的间隔（ADR-0050：扫码后先问一次，之后每 10 分钟一次；没问到时更快重问，见 `spawn_poll`）。
pub(crate) const OUTCOME_POLL: Duration = Duration::from_secs(600);
