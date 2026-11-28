//! 对联邦和隧道的接口：入站名单、出站查找、证书校验材料、转发状态。

use std::collections::BTreeSet;
use std::sync::Arc;

use iroh::{EndpointAddr, EndpointId};
use tokio::sync::watch;
use wonton_protocol::PeerStatus;
use wonton_tunnel::Allowlist;
use wonton_tunnel::federation::{
    PeerStates, PeerStatus as FederationStatus, PeerVerification, PinnedPeerCert,
};
use wonton_tunnel::peer_pair::PeerIdentity;

use super::Peers;
use super::pin::{PinState, window_expired};
use super::record::{Phase, now_ms};

impl Peers {
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
}
