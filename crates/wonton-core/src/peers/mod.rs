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
//! - 证书（ADR-0112）：配对成立时记成待钉（窗口起点是当时的核心版本），两端经配对通道互换自签叶证书，
//!   收到就钉；回执后只留新的，旧的最多留 7 天。删配对时 pin 一起删。
//!
//! 联邦转发用的接口：[`Peers::server_allowlist`]（入站名单）、[`Peers::lookup`]（服务器名 → endpoint ID，
//! 核心层的“一个服务器名只会转发到配对时绑定的 endpoint ID”）、[`Peers::paired_names`]（已配对的服务器名，
//! 给 Tuwunel 配置的订阅）。

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use iroh::{EndpointAddr, EndpointId};
use serde::{Deserialize, Serialize};
use tokio::sync::watch;
use wonton_protocol::{PeerRequestRow, PeerRow, PeerStatus, PeersView, RejectedPeerRow};
use wonton_tunnel::Allowlist;
use wonton_tunnel::federation::{
    PeerStates, PeerStatus as FederationStatus, PeerVerification, PinnedPeerCert,
};
use wonton_tunnel::peer_pair::PeerIdentity;

use crate::plugin::languages::Languages;
use crate::tunnel::Tunnel;

mod code;
pub mod pairing;
mod pin;
mod record;
use pin::prune_expired_pins;
pub use pin::{OwnCertificate, PeerPin, PinState, load_own_certificate, window_expired};
#[cfg(test)]
mod peers_tests;
mod scanning;

pub(crate) use record::{HOUR_MS, MAX_PENDING, MAX_PER_HOUR, endpoint_addr, now_ms};
pub use record::{
    OwnIdentity, OwnerNotices, PeerRecord, PeerRequest, PeerStoreError, Phase, RejectedRequest,
};

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
    /// 公网访问总开关（ADR-0090）：关着时已配对的行报“公网访问已关闭”。
    public_access: Mutex<Option<Arc<crate::entry_access::EntryAccess>>>,
    /// 主人在“Wonton”私聊里的提醒。
    notices: Option<Arc<dyn OwnerNotices>>,
    /// 主人的显示名，每次握手现取：主人改了资料，下次配对用的就是新名字。
    owner_display_name: Arc<dyn Fn() -> String + Send + Sync>,
    /// 主人的语言，说明和提醒按它写（ADR-0043）。
    languages: Arc<dyn Fn() -> Languages + Send + Sync>,
    /// 本机证书的来源（ADR-0112）：配对成立后发给对方。
    own_certificate: Mutex<Option<OwnCertificate>>,
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
        let version = crate::build_info::version();
        // 老记录没有窗口起点：从这次启动的核心版本补上（ADR-0112 §5b）。离线期间过了宽限期的
        // 旧 pin 顺手丢掉，只留最新一张。
        let mut stored_changed = false;
        for record in stored.peers.values_mut() {
            if let PinState::Pending { since_version } = &mut record.pin_state
                && since_version.is_empty()
            {
                *since_version = crate::build_info::version().to_owned();
                stored_changed = true;
            }
            stored_changed |= prune_expired_pins(&mut record.peer_pin);
            if let PinState::Pending { since_version } = &record.pin_state
                && !since_version.is_empty()
                && window_expired(since_version, version)
            {
                tracing::warn!(
                    server = %record.server_name,
                    since_version,
                    version,
                    "待钉窗口已到期，对端还没升级；握手仍按证书名字放行"
                );
            }
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
            public_access: Mutex::new(None),
            notices,
            owner_display_name,
            languages,
            own_certificate: Mutex::new(None),
            #[cfg(test)]
            changes: watch::channel(0).0,
        });
        if created_token || stored_changed {
            let inner = peers.lock();
            if let Err(e) = peers.save(&inner) {
                tracing::error!(error = %e, "配对记录写不进去");
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
    /// 名单变化后由订阅方改 Tuwunel 的配置并让它重读（管理房间命令，fed-tuwunel-config 的任务）。
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

    /// 出站 TLS 的对端校验材料（ADR-0112）：钉住的叶（轮换期新旧都在），或按窗口如实报“待钉”。
    ///
    /// 还没落 pin 的旧记录、以及刚配对还没收到对方证书的记录按 §5b 走待钉：带上窗口起点和是否到期
    /// （到期仍只校名字放行，不直接断），不恒 `None`。`Pinned` 的每条记录都必须自洽（DER 和 SPKI 指纹
    /// 对得上），一条坏数据就 fail-closed 成空根库，不让坏记录悄悄少一张 pin。
    pub fn tls_verification(&self, server_name: &str) -> Option<PeerVerification> {
        let peers = self.lock();
        let record = peers.peers.get(server_name)?;
        if record.phase != Phase::Paired {
            return None;
        }
        match &record.pin_state {
            PinState::Pending { since_version } => Some(PeerVerification::Pending {
                since_version: (!since_version.is_empty()).then(|| since_version.clone()),
                expired: record
                    .pin_state
                    .window_expired(crate::build_info::version()),
            }),
            PinState::Pinned => {
                let mut pins = Vec::with_capacity(record.peer_pin.len());
                let mut broken = false;
                for pin in &record.peer_pin {
                    match pin
                        .certificate
                        .der_bytes()
                        .zip(pin.certificate.spki_sha256_bytes())
                        .map(|(der, spki)| PinnedPeerCert::from_parts(der, spki))
                    {
                        Some(Ok(pin)) => pins.push(pin),
                        Some(Err(error)) => {
                            tracing::error!(server = %server_name, %error, "钉住的证书坏了，fail-closed");
                            broken = true;
                        }
                        None => {
                            tracing::error!(server = %server_name, "钉住的证书编码坏了，fail-closed");
                            broken = true;
                        }
                    }
                }
                if broken {
                    pins.clear();
                }
                Some(PeerVerification::Pinned(pins))
            }
        }
    }

    /// 这次的配对握手里要发给对方的身份。主人的显示名现取，主人改资料后不用重启核心。
    pub(crate) fn own_identity(&self) -> PeerIdentity {
        self.own.to_wire(&self.tunnel, (self.owner_display_name)())
    }

    /// 一台服务器在联系人看来的样子（ADR-0050）：已配对时是转发组件给的状态，没配对或配对已经移除时为空。
    pub fn contact_reach(&self, server_name: &str) -> Option<PeerStatus> {
        let paired = self
            .lock()
            .peers
            .get(server_name)
            .is_some_and(|record| record.phase == Phase::Paired);
        paired.then(|| self.forwarded_status(server_name))
    }

    /// 接上公网访问总开关：关着时已配对的行报公网访问已关闭（ADR-0050 的状态表）。
    pub fn attach_public_access(&self, access: Arc<crate::entry_access::EntryAccess>) {
        *self.public_access.lock().expect("公网开关锁中毒") = Some(access);
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

    /// 窗口已到期、还没拿到对方证书的已配对服务器（ADR-0112 §5b）：如实报“对端还没升级”。
    pub fn expired_pending(&self) -> Vec<String> {
        self.expired_pending_on(crate::build_info::version())
    }

    /// 同 [`Self::expired_pending`]，按给出的当前版本判（测试用，免得开发构建的版本不可排序）。
    pub fn expired_pending_on(&self, current_version: &str) -> Vec<String> {
        let inner = self.lock();
        inner
            .peers
            .values()
            .filter(|record| {
                if record.phase != Phase::Paired || !record.peer_pin.is_empty() {
                    return false;
                }
                let PinState::Pending { since_version } = &record.pin_state else {
                    return false;
                };
                !since_version.is_empty() && window_expired(since_version, current_version)
            })
            .map(|record| record.server_name.clone())
            .collect()
    }

    /// 转发组件报“已连上”：记下时刻。同一段连接上反复报（数据往返也确认连接）时，同一毫秒不重复写盘。
    /// 还没拿到对方证书的已配对记录顺手再送一次本机证书：配对后那几发送不到（对方离线、还在重启）时，
    /// 等下一次连上补（ADR-0112 §5b 的送达不靠运气）。
    fn note_connected(self: &Arc<Self>, server_name: &str) {
        let mut inner = self.lock();
        let Some(record) = inner.peers.get_mut(server_name) else {
            return;
        };
        let now = now_ms();
        if record.last_connected_ms == Some(now) {
            return;
        }
        record.last_connected_ms = Some(now);
        let resend = record.phase == Phase::Paired && record.peer_pin.is_empty();
        if let Err(e) = self.save(&inner) {
            tracing::error!(error = %e, "配对记录写不进去");
        }
        drop(inner);
        if resend {
            let peers = self.clone();
            let server_name = server_name.to_owned();
            tokio::spawn(async move { peers.send_certificate(&server_name, false).await });
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
        let probe = match states.borrow().get(server_name) {
            Some(FederationStatus::Connected) => PeerStatus::Connected,
            Some(FederationStatus::Offline { .. }) => PeerStatus::Offline,
            Some(FederationStatus::RemovedByPeer) => PeerStatus::RemovedByPeer,
            None => PeerStatus::Checking,
        };
        // 自己的公网访问关着、而且本机有公网名字时，行如实说“公网访问已关闭”（ADR-0050 的状态表）：
        // 私用名的安装没有公网入口，照旧报转发组件的状态；配对经 8449 的联邦仍照常工作（ADR-0090:52）。
        let public_off = !crate::matrix::federation_config::is_private(&self.own.server_name)
            && self
                .public_access
                .lock()
                .expect("公网开关锁中毒")
                .as_ref()
                .is_some_and(|access| !access.enabled());
        match probe {
            // 还没探活过就不知道经公网还是经配对前端，保持“正在确认”。
            PeerStatus::Checking => PeerStatus::Checking,
            // 对方删了配对与总开关无关，照报。
            PeerStatus::RemovedByPeer => PeerStatus::RemovedByPeer,
            _ if public_off => PeerStatus::PublicAccessOff,
            other => other,
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
        let text = format!(
            "{}\n{}",
            crate::l10n::text(
                &langs,
                "peers-new-request",
                &[
                    ("owner", who.into()),
                    ("server", request.server_name.as_str().into()),
                ],
            ),
            wonton_protocol::AppLink::Peers.to_url()
        );
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
