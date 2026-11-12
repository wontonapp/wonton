//! 服务器配对（ADR-0050）：主人出码、对方扫码、出码方点同意，两台服务器的 Matrix 联邦走 iroh。
//!
//! - 配对记录存 `core/peers.json`，服务器名和对方 endpoint ID 的绑定只在配对时定下、以后不变。
//! - 出码：一次性密钥，和确认卡片同时长有效，同一时间只有一张。码是 `wonton://peer?…`，格式由核心定，壳只显示。
//! - 扫码：主人的设备把码交给核心（`/_wonton/api/peers`），核心经隧道的 `wonton/peer/0` 连出码方；成功后
//!   记成“等对方同意”。
//! - 同意：对方扫码时，出码方在“Wonton”私聊里发确认卡片（ADR-0016 的 `ConfirmationCard.peer_request`）。
//!   同意后两边记成已配对；拒绝（或卡片过期）后通知扫码方，扫码方的那条记录删掉。
//! - 删配对：记录删掉、联邦名单撤掉、通知对方一次（尽力而为）。
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

use iroh::{EndpointAddr, EndpointId};
use serde::{Deserialize, Serialize};
use tokio::sync::watch;
use wonton_protocol::{
    Answer, CardChoice, CardLine, ConfirmationCard, ConfirmationState, PeerCode, PeerRequest,
    PeerRow, PeerStatus, PeersView, SecretString,
};
use wonton_tunnel::Allowlist;
use wonton_tunnel::federation::{PeerStates, PeerStatus as FederationStatus};
use wonton_tunnel::peer_pair::{PeerIdentity, PeerPairingCode};

use crate::mcp::Confirmations;
use crate::plugin::languages::Languages;
use crate::tunnel::Tunnel;

mod code;
pub mod pairing;
#[cfg(test)]
mod peers_tests;

/// 配对的进展。列表上“等对方同意”的行只对 [`Phase::AwaitingPeer`] 显示（ADR-0050 的状态表）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// 已配对：联邦名单里有它。
    Paired,
    /// 我扫了对方，对方的主人还没同意。
    AwaitingPeer,
    /// 对方扫了我，我的主人还没回答卡片。不出现在列表里，卡片就是它的展示。
    AwaitingOwner,
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
    /// 还在等结果的记录过了这个时刻（Unix 毫秒）就当对方没有回应，删掉。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_expires_at_ms: Option<u64>,
}

impl PeerRecord {
    /// 回连这一端用的地址。
    pub(crate) fn endpoint_addr(&self) -> EndpointAddr {
        let mut addr = EndpointAddr::new(self.endpoint_id);
        for relay in &self.relays {
            if let Ok(relay) = relay.parse() {
                addr = addr.with_relay_url(relay);
            }
        }
        for ip in &self.addrs {
            addr = addr.with_ip_addr(*ip);
        }
        addr
    }
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

/// 服务器配对：记录、出码、扫码、同意卡片、删配对，以及联邦名单。
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
    /// 主人的回答从确认卡片来（ADR-0016）。
    confirmations: Arc<Confirmations>,
    /// 主人的显示名，每次握手现取：主人改了资料，下次配对用的就是新名字。
    owner_display_name: Arc<dyn Fn() -> String + Send + Sync>,
    /// 主人的语言，卡片和说明按它写（ADR-0043）。
    languages: Arc<dyn Fn() -> Languages + Send + Sync>,
    inner: Mutex<Inner>,
}

#[derive(Debug, Default)]
struct Inner {
    /// 服务器名 → 记录。
    peers: BTreeMap<String, PeerRecord>,
    /// 主人当前出的码，同一时间一张。
    code: Option<code::IssuedCode>,
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
    /// 读取记录；文件不在时从空表开始。重启后还在等我们回答的记录一并删掉：卡片没有留下来，回答永远不会来。
    pub fn open(
        path: impl Into<PathBuf>,
        own: OwnIdentity,
        tunnel: Tunnel,
        confirmations: Arc<Confirmations>,
        owner_display_name: Arc<dyn Fn() -> String + Send + Sync>,
        languages: Arc<dyn Fn() -> Languages + Send + Sync>,
    ) -> Result<Arc<Self>, PeerStoreError> {
        let path = path.into();
        let peers = read_records(&path)?;
        let paired: BTreeSet<String> = peers
            .values()
            .filter(|record| record.phase == Phase::Paired)
            .map(|record| record.server_name.clone())
            .collect();
        let server_allowlist = Allowlist::new(
            peers
                .values()
                .filter(|record| record.phase == Phase::Paired)
                .map(|record| record.endpoint_id),
        );
        let (names, _) = watch::channel(paired);
        Ok(Arc::new(Self {
            inner: Mutex::new(Inner { peers, code: None }),
            path,
            own,
            tunnel,
            names,
            server_allowlist,
            forwarder: Mutex::new(None),
            confirmations,
            owner_display_name,
            languages,
        }))
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().expect("配对表锁中毒")
    }

    /// 整份重写记录：先写临时文件再改名，避免写到一半断电留下损坏的记录（照 `devices.json` 的做法）。
    /// 调用方持着 `self.lock()`，这把锁同时串行化写盘。
    fn save(&self, peers: &BTreeMap<String, PeerRecord>) -> Result<(), PeerStoreError> {
        let io = |source| PeerStoreError::Io {
            path: self.path.clone(),
            source,
        };
        let stored = Stored {
            peers: peers.values().cloned().collect(),
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
        std::fs::rename(&tmp, &self.path).map_err(io)
    }

    /// 测试用：隧道入口（endpoint、设备登记表和设备配对处理方都在里面）。
    #[cfg(test)]
    pub(crate) fn tunnel(&self) -> &Tunnel {
        &self.tunnel
    }

    /// 测试用：出一张已经过期的码，验出码方的过期拒绝。
    #[cfg(test)]
    pub fn issue_code_expired_for_test(&self) -> wonton_protocol::PeerCode {
        let mut inner = self.lock();
        let mut issued = code::issue(code::CODE_TTL);
        issued.expires_at_ms = now_ms().saturating_sub(1);
        let expires_at_ms = issued.expires_at_ms;
        let secret = issued.secret.clone();
        inner.code = Some(issued);
        let uri = wonton_tunnel::peer_pair::PeerPairingCode {
            server_id: self.tunnel.endpoint.id(),
            relay_urls: self.tunnel.relays().iter().map(|r| r.url.clone()).collect(),
            direct_addrs: self.tunnel.direct_addrs(),
            server_name: self.own.server_name.clone(),
            secret,
        }
        .to_uri();
        wonton_protocol::PeerCode {
            code: wonton_protocol::SecretString::new(uri),
            expires_at_ms,
        }
    }

    /// 确认卡片等回答的时间，也是“等对方同意”的记录过期的时间。
    pub(crate) fn ttl(&self) -> Duration {
        self.confirmations.ttl()
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
        if let Err(e) = self.save(&inner.peers) {
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
            if let Err(e) = self.save(&inner.peers) {
                tracing::error!(error = %e, "配对记录写不进去");
            }
            endpoint
        };
        self.retract(server_name, endpoint);
        tracing::info!(server = %server_name, "对方服务器拒绝了本机，记录标成对方已断开");
    }

    /// 主人看到的服务器列表和当前出的码。还在等我们回答的记录不列（卡片就是它的展示），
    /// 过了期的“等对方同意”就地删掉。
    pub fn view(&self, langs: &Languages) -> PeersView {
        let rows = {
            let inner = self.lock();
            let mut peers = inner.peers.clone();
            let now = now_ms();
            peers.retain(|_, record| {
                record
                    .pending_expires_at_ms
                    .is_none_or(|expires| expires > now)
            });
            peers
                .values()
                .filter(|record| record.phase != Phase::AwaitingOwner)
                .map(|record| self.row(record, langs))
                .collect::<Vec<_>>()
        };
        PeersView {
            peers: rows,
            requests: Vec::new(),
            rejected: Vec::new(),
            code: self.active_code(),
        }
    }

    /// 还没过期的出码。已经配上或对方扫了我们的码之后，码作废，别让主人再扫一个没用的。
    fn active_code(&self) -> Option<PeerCode> {
        let inner = self.lock();
        let issued = inner.code.as_ref()?;
        if issued.expires_at_ms <= now_ms() {
            return None;
        }
        let waiting = inner
            .peers
            .values()
            .any(|record| record.phase == Phase::AwaitingOwner);
        (!waiting).then(|| PeerCode {
            code: SecretString::new(
                PeerPairingCode {
                    server_id: self.tunnel.endpoint.id(),
                    relay_urls: self.tunnel.relays().iter().map(|r| r.url.clone()).collect(),
                    direct_addrs: self.tunnel.direct_addrs(),
                    server_name: self.own.server_name.clone(),
                    secret: issued.secret.clone(),
                }
                .to_uri(),
            ),
            expires_at_ms: issued.expires_at_ms,
        })
    }

    fn row(&self, record: &PeerRecord, langs: &Languages) -> PeerRow {
        let status = match record.phase {
            Phase::AwaitingPeer => PeerStatus::AwaitingApproval,
            Phase::RemovedByPeer => PeerStatus::RemovedByPeer,
            Phase::Paired | Phase::AwaitingOwner => self.forwarded_status(&record.server_name),
        };
        let last_connected_ms = record.last_connected_ms;
        PeerRow {
            server_name: record.server_name.clone(),
            owner_user_id: record.owner_user_id.clone(),
            owner_display_name: record.owner_display_name.clone(),
            status,
            last_connected_ms,
            detail: Some(self.detail(record, langs)),
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
        let id = record.endpoint_id.to_string()[..8].to_owned();
        match self.last_error(&record.server_name) {
            Some(error) => crate::l10n::text(
                langs,
                "peers-endpoint-detail",
                &[("id", id.into()), ("error", error.into())],
            ),
            None => crate::l10n::text(langs, "peers-endpoint", &[("id", id.into())]),
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

    /// 卡片上的确认请求：标题和说明按主人的语言写（ADR-0050 的“某某的服务器想和你的服务器连接”）。
    pub(crate) fn peer_request_card(
        &self,
        requester: &PeerIdentity,
        pending_expires_at_ms: u64,
    ) -> ConfirmationCard {
        let langs = (self.languages)();
        let title = crate::l10n::text(
            &langs,
            "peers-request-title",
            &[("owner", requester.owner_display_name.as_str().into())],
        );
        ConfirmationCard {
            id: crate::secret::random_token(),
            state: ConfirmationState::Pending,
            title,
            lines: vec![CardLine {
                text: crate::l10n::text(
                    &langs,
                    "peers-request-server",
                    &[("server", requester.server_name.as_str().into())],
                ),
                emphasis: false,
            }],
            choices: approve_reject(&langs),
            expires_at_ms: pending_expires_at_ms,
            window: None,
            plugin_id: None,
            peer_request: Some(PeerRequest {
                server_name: requester.server_name.clone(),
                owner_display_name: requester.owner_display_name.clone(),
            }),
        }
    }
}

/// 卡片上的“连接”和“拒绝”按钮。回答按 [`Answer`] 匹配，不需要 `choice_id`。
fn approve_reject(langs: &Languages) -> Vec<CardChoice> {
    vec![
        CardChoice {
            answer: Answer::Approve,
            label: crate::l10n::text(langs, "peers-connect", &[]),
            choice_id: None,
        },
        CardChoice {
            answer: Answer::Reject,
            label: crate::l10n::text(langs, "peers-reject", &[]),
            choice_id: None,
        },
    ]
}

fn read_records(path: &Path) -> Result<BTreeMap<String, PeerRecord>, PeerStoreError> {
    let mut peers = BTreeMap::new();
    match std::fs::read_to_string(path) {
        Ok(text) => {
            let stored: Stored =
                serde_json::from_str(&text).map_err(|e| PeerStoreError::Corrupt {
                    path: path.to_owned(),
                    message: e.to_string(),
                })?;
            for record in stored.peers {
                if record.phase == Phase::AwaitingOwner {
                    tracing::info!(
                        server = %record.server_name,
                        "核心重启过，对方还在等我们的同意；这条记录删掉，请对方重新扫码"
                    );
                    continue;
                }
                peers.insert(record.server_name.clone(), record);
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(source) => {
            return Err(PeerStoreError::Io {
                path: path.to_owned(),
                source,
            });
        }
    }
    Ok(peers)
}

/// `core/peers.json` 的形状。
#[derive(Serialize, Deserialize)]
struct Stored {
    peers: Vec<PeerRecord>,
}
