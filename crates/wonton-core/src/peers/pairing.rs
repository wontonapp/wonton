//! 配对的业务流程：出码、扫码、同意卡片、删配对，以及隧道上 [`PeerPairHandler`] 的实现。
//!
//! 删配对分两种：主人移除（记录真删，通知对方一次，尽力而为）和对方删了（记录留着、标成对方已断开，
//! 等主人移除）。重新配对时，标过的记录被新记录替换。
//!
//! [`PeerPairHandler`] 的方法由隧道在连接上同步调用，配对的后续（等卡片回答、送结果）是异步的，
//! 所以真正的方法挂在 `Arc<Peers>` 上，由 [`PeerPairing`] 包装后交给隧道，后续任务克隆这个 Arc。

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use iroh::EndpointId;
use wonton_protocol::{ConfirmationState, PeerError};
use wonton_tunnel::peer_pair::{
    PeerIdentity, PeerPairHandler, PeerPairSecret, PeerRejectReason, PeerReply, deliver_decision,
    notify_removed, scan,
};

use super::{PeerRecord, Peers, Phase, code, now_ms};
use crate::plugin::PluginId;

/// 确认卡片里的调用方：配对不是插件发起的，用一个固定的伪 ID 占住“同时最多几张卡片”的名额。
fn caller() -> PluginId {
    PluginId::parse("core.peers").expect("固定的调用方 ID 总能解析")
}

/// “等对方同意”的记录比卡片多留这么久，对方核心稍慢一点也等得到。
const PENDING_MARGIN: Duration = Duration::from_secs(60);

/// 配对结果重试送几次：对方暂时忙或刚好在重启时还能送到。
const DECISION_TRIES: u32 = 3;

/// 两次重试之间隔多久。
const DECISION_RETRY_PAUSE: Duration = Duration::from_secs(3);

/// 扫码的回应和卡片答完之后的后续任务。
type FollowUp = Pin<Box<dyn Future<Output = ()> + Send>>;

/// 隧道用的包装：同步地应答，把异步的后续交给 tokio。挂在 iroh router 上：
/// `PeerPairProtocol::new(Arc::new(PeerPairing(peers.clone())))`。
#[derive(Debug)]
pub struct PeerPairing(pub Arc<Peers>);

impl PeerPairHandler for PeerPairing {
    fn scanned(
        &self,
        scanner: EndpointId,
        identity: &PeerIdentity,
        secret: &PeerPairSecret,
    ) -> PeerReply {
        let (reply, followup) = self.0.on_scanned(scanner, identity, secret);
        if let Some(followup) = followup {
            tokio::spawn(followup);
        }
        reply
    }

    fn expecting_decision(&self, from: &EndpointId) -> bool {
        self.0.on_expecting_decision(from)
    }

    fn decision(&self, from: EndpointId, approved: bool, identity: &PeerIdentity) {
        if let Some(followup) = self.0.on_decision(from, approved, identity) {
            tokio::spawn(followup);
        }
    }

    fn removed(&self, from: EndpointId, server_name: &str) {
        self.0.on_removed(from, server_name);
    }
}

/// 配对结果重试送几次，送到就算。
async fn deliver_with_retries(
    endpoint: &iroh::Endpoint,
    addr: iroh::EndpointAddr,
    approved: bool,
    identity: PeerIdentity,
) -> bool {
    for attempt in 1..=DECISION_TRIES {
        match deliver_decision(endpoint, addr.clone(), approved, identity.clone()).await {
            Ok(()) => return true,
            Err(e) if attempt < DECISION_TRIES => {
                tracing::debug!(attempt, error = %e, "配对结果没有送到，稍后再试");
                tokio::time::sleep(DECISION_RETRY_PAUSE).await;
            }
            Err(e) => tracing::debug!(error = %e, "配对结果没有送到"),
        }
    }
    false
}

/// 出码方的拒绝原因映射成配对接口的错误类别（`wonton-protocol` 的 `PeerError`）。
fn reject_to_peer_error(reason: PeerRejectReason) -> PeerError {
    match reason {
        PeerRejectReason::Expired => PeerError::CodeExpired,
        PeerRejectReason::AlreadyPaired => PeerError::AlreadyPaired,
        PeerRejectReason::SelfPairing => PeerError::SelfPairing,
        // 没发过、用过、请求不合规：码不可信，一律按“码读不懂”说清。
        PeerRejectReason::UnknownCode | PeerRejectReason::Used | PeerRejectReason::BadRequest => {
            PeerError::CodeInvalid
        }
        // 出码方自己出错、比这个核心新的原因：如实归到“其他”。
        PeerRejectReason::ServerError | PeerRejectReason::Other => PeerError::Other,
    }
}

fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

impl Peers {
    /// 主人出一张新码：上一张作废，同一时间只有一张。
    pub fn issue_code(&self) -> wonton_protocol::PeerCode {
        let mut inner = self.lock();
        let issued = code::issue(code::CODE_TTL);
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
        tracing::info!("主人出了一张服务器配对码");
        wonton_protocol::PeerCode {
            code: wonton_protocol::SecretString::new(uri),
            expires_at_ms,
        }
    }

    /// 主人的设备扫来一个码：连出码方，成功后记成“等对方同意”。
    pub async fn scan(self: &Arc<Self>, code_text: &str) -> Result<(), PeerError> {
        let code = wonton_tunnel::peer_pair::PeerPairingCode::parse(code_text)
            .map_err(|_| PeerError::CodeInvalid)?;
        // 扫了自己：不用连，直接说清。
        if code.server_name == self.own.server_name || code.server_id == self.tunnel.endpoint.id() {
            return Err(PeerError::SelfPairing);
        }
        {
            let mut inner = self.prune();
            match inner.peers.get(&code.server_name) {
                Some(record) if record.phase == Phase::Paired => {
                    return Err(PeerError::AlreadyPaired);
                }
                // 对方删了我们、它的记录已经没了：新配对替换这条，旧 endpoint 从入站名单撤掉。
                Some(record) if record.phase == Phase::RemovedByPeer => {
                    let old = record.endpoint_id;
                    self.retract(&code.server_name, old);
                }
                _ => {}
            }
            let record = PeerRecord {
                server_name: code.server_name.clone(),
                endpoint_id: code.server_id,
                relays: code.relay_urls.iter().map(|url| url.to_string()).collect(),
                addrs: code.direct_addrs.clone(),
                owner_user_id: String::new(),
                owner_display_name: String::new(),
                paired_at_ms: 0,
                phase: Phase::AwaitingPeer,
                last_connected_ms: None,
                pending_expires_at_ms: Some(now_ms() + millis(self.ttl() + PENDING_MARGIN)),
            };
            inner
                .peers
                .insert(record.server_name.clone(), record.clone());
            self.save(&inner.peers)
                .map_err(|e| storage_failed(e, "扫码的记录写不进去"))?;
        }
        let code_server_name = code.server_name.clone();
        let identity = self.own_identity();
        match scan(&self.tunnel.endpoint, code, identity).await {
            // 等对方同意：把对方的身份记上。
            Ok(PeerReply::AwaitingApproval { identity }) => {
                if identity.server_name != code_server_name {
                    // 出码方报的服务器名和码里的对不上：码不可信，扔掉。
                    self.forget_pending(&code_server_name);
                    return Err(PeerError::CodeInvalid);
                }
                let mut inner = self.lock();
                let updated = inner.peers.get_mut(&code_server_name).map(|record| {
                    record.owner_user_id = identity.owner_user_id.clone();
                    record.owner_display_name = identity.owner_display_name.clone();
                    record.server_name.clone()
                });
                match updated {
                    Some(name) => {
                        self.save(&inner.peers)
                            .map_err(|e| storage_failed(e, "扫码的记录写不进去"))?;
                        tracing::info!(server = %name, "扫了对方的服务器，等对方同意");
                        Ok(())
                    }
                    // 记录在等待对方回应时被过期清掉了：当作没有扫过，让主人重新扫。
                    None => Err(PeerError::PeerUnreachable),
                }
            }
            // 出码方拒绝：按原因说清。
            Ok(PeerReply::Error { reason, .. }) => {
                self.forget_pending(&code_server_name);
                Err(reject_to_peer_error(reason))
            }
            Ok(PeerReply::Ok) => {
                self.forget_pending(&code_server_name);
                Err(PeerError::CodeInvalid)
            }
            // 连不上出码方。
            Err(e) => {
                self.forget_pending(&code_server_name);
                tracing::info!(error = %e, "连不上出码方");
                Err(PeerError::PeerUnreachable)
            }
        }
    }

    /// 主人移除一台服务器的配对：记录删掉、名单撤掉，通知对方（尽力而为，不挡着返回）。
    /// 对方删了我们时，那边已经没有记录，收到通知也只当没这回事，不算错。
    pub async fn remove(
        self: &Arc<Self>,
        server_name: &str,
    ) -> Result<bool, super::PeerStoreError> {
        let record = {
            let mut inner = self.lock();
            let Some(record) = inner.peers.remove(server_name) else {
                return Ok(false);
            };
            self.save(&inner.peers)?;
            self.retract(server_name, record.endpoint_id);
            record.clone()
        };
        tracing::info!(server = %server_name, "移除了和对方服务器的配对");
        // 对方删了我们、这边只是把记录清掉时，它的记录已经没了：通知照样发，送到也只当没这回事。
        if record.phase != Phase::RemovedByPeer {
            let peers = self.clone();
            let addr = record.endpoint_addr();
            let own = self.own.server_name.clone();
            tokio::spawn(async move {
                // 一次就够：对方离线时，它以后在联邦握手被拒时知道（ADR-0050 的状态表）。
                if let Err(e) = notify_removed(&peers.tunnel.endpoint, addr, &own).await {
                    tracing::info!(error = %e, "移除配对的通知没有送到对方");
                }
            });
        }
        Ok(true)
    }

    /// 删掉还在等结果的扫码记录（对方拒绝了、连不上或过期）。
    pub(crate) fn forget_pending(self: &Arc<Self>, server_name: &str) {
        let mut inner = self.lock();
        let pending = inner
            .peers
            .get(server_name)
            .is_some_and(|record| record.phase == Phase::AwaitingPeer);
        if pending {
            inner.peers.remove(server_name);
            if let Err(e) = self.save(&inner.peers) {
                tracing::error!(error = %e, "配对记录写不进去");
            }
        }
    }

    /// 名单维护：联邦名单和已配对服务器名里撤掉。
    pub(super) fn retract(&self, server_name: &str, endpoint_id: EndpointId) {
        self.server_allowlist.revoke(&endpoint_id);
        self.names
            .send_if_modified(|names| names.remove(server_name));
    }

    /// 过了期的“等对方同意”就地删掉。
    fn prune(&self) -> std::sync::MutexGuard<'_, super::Inner> {
        let mut inner = self.lock();
        let now = now_ms();
        let expired: Vec<String> = inner
            .peers
            .values()
            .filter(|record| {
                record
                    .pending_expires_at_ms
                    .is_some_and(|expires| expires <= now)
            })
            .map(|record| record.server_name.clone())
            .collect();
        for name in expired {
            inner.peers.remove(&name);
        }
        inner
    }

    /// 同意：记成已配对，把结果送给对方（对方也记成已配对），结果送出后再让转发组件开始探活。
    ///
    /// 顺序要紧：对方一收下结果就会回送一条结果并开始探活，所以要先把对方放进本机的入站名单，再送结果；
    /// 本机自己的探活由已配对名单触发，等结果送出（或确认送不到）之后再加，否则会赶在对方把本机放进
    /// 名单之前探活，第一次就被拒。
    async fn approve(
        self: &Arc<Self>,
        server_name: &str,
        scanner: EndpointId,
        identity: PeerIdentity,
    ) {
        let (_, addr) = {
            let mut inner = self.prune();
            let Some(record) = inner.peers.get_mut(server_name) else {
                return;
            };
            if record.phase == Phase::Paired {
                return; // 双方同时扫过对方的码、或对方的结果已经先到。
            }
            let previous = record.endpoint_id;
            record.phase = Phase::Paired;
            record.endpoint_id = scanner;
            record.relays = identity.relays.clone();
            record.addrs = identity.addrs.clone();
            record.owner_user_id = identity.owner_user_id.clone();
            record.owner_display_name = identity.owner_display_name.clone();
            record.paired_at_ms = now_ms();
            record.pending_expires_at_ms = None;
            if let Err(e) = self.save(&inner.peers) {
                tracing::error!(error = %e, "配对记录写不进去");
                return;
            }
            if previous != scanner {
                self.server_allowlist.revoke(&previous);
            }
            self.server_allowlist.allow(scanner);
            (previous, identity.endpoint_addr(scanner))
        };
        tracing::info!(server = %server_name, "同意了和对方服务器的配对");
        let own = self.own_identity();
        if !deliver_with_retries(&self.tunnel.endpoint, addr, true, own).await {
            tracing::warn!(
                server = %server_name,
                "配对结果没有送到对方：对方的主人重新扫码即可接上"
            );
        }
        let still_paired = self
            .lock()
            .peers
            .get(server_name)
            .is_some_and(|record| record.phase == Phase::Paired);
        if still_paired {
            self.names
                .send_if_modified(|names| names.insert(server_name.to_owned()));
        }
    }

    /// 拒绝（或卡片过期）：自己的记录删掉，把结果送给对方。
    async fn reject(
        self: &Arc<Self>,
        server_name: &str,
        scanner: EndpointId,
        identity: PeerIdentity,
    ) {
        {
            let mut inner = self.lock();
            let matches = inner.peers.get(server_name).is_some_and(|record| {
                record.phase == Phase::AwaitingOwner && record.endpoint_id == scanner
            });
            if matches {
                inner.peers.remove(server_name);
                if let Err(e) = self.save(&inner.peers) {
                    tracing::error!(error = %e, "配对记录写不进去");
                }
            }
        }
        tracing::info!(server = %server_name, "拒绝了和对方服务器的配对");
        let own = self.own_identity();
        let addr = identity.endpoint_addr(scanner);
        deliver_with_retries(&self.tunnel.endpoint, addr, false, own).await;
    }
}

fn storage_failed(error: super::PeerStoreError, message: &str) -> PeerError {
    tracing::error!(error = %error, "{message}");
    PeerError::Other
}

/// 等主人回答的一次扫码：卡片答完就知道要配哪台。
struct WaitingScan {
    server_name: String,
    scanner: EndpointId,
    identity: PeerIdentity,
}

impl Peers {
    /// 有人扫了我们的码：验码，记下“等主人同意”，发确认卡片。返回给对方的回应和等回答的后续。
    pub(crate) fn on_scanned(
        self: &Arc<Self>,
        scanner: EndpointId,
        identity: &PeerIdentity,
        secret: &PeerPairSecret,
    ) -> (PeerReply, Option<FollowUp>) {
        let error =
            |reason: PeerRejectReason, message: String| PeerReply::Error { reason, message };
        if identity.server_name == self.own.server_name || scanner == self.tunnel.endpoint.id() {
            return (
                error(PeerRejectReason::SelfPairing, "扫了自己的服务器".to_owned()),
                None,
            );
        }
        let mut inner = self.prune();
        match inner.peers.get(&identity.server_name) {
            Some(record) if record.phase == Phase::Paired => {
                return (
                    error(
                        PeerRejectReason::AlreadyPaired,
                        format!("和 {} 已经配对过", identity.server_name),
                    ),
                    None,
                );
            }
            // 对方删了我们、它的记录已经没了：新配对替换这条，旧 endpoint 从入站名单撤掉。
            Some(record) if record.phase == Phase::RemovedByPeer => {
                let old = record.endpoint_id;
                self.retract(&identity.server_name, old);
            }
            _ => {}
        }
        if let Err(reject) = code::verify(&inner.code, secret) {
            return (error(reject.reason, reject.message), None);
        }
        let expires_at_ms = now_ms() + millis(self.ttl());
        let record = PeerRecord {
            server_name: identity.server_name.clone(),
            endpoint_id: scanner,
            relays: identity.relays.clone(),
            addrs: identity.addrs.clone(),
            owner_user_id: identity.owner_user_id.clone(),
            owner_display_name: identity.owner_display_name.clone(),
            paired_at_ms: 0,
            phase: Phase::AwaitingOwner,
            last_connected_ms: None,
            pending_expires_at_ms: Some(expires_at_ms),
        };
        inner
            .peers
            .insert(record.server_name.clone(), record.clone());
        if let Err(e) = self.save(&inner.peers) {
            tracing::error!(error = %e, "配对记录写不进去");
            return (
                error(
                    PeerRejectReason::ServerError,
                    "服务器写配对记录失败，稍后再试".to_owned(),
                ),
                None,
            );
        }
        // 一次性：记录落盘了才作废，写不进去时码还留着。
        inner.code = None;
        let card = self.peer_request_card(identity, expires_at_ms);
        let waiting = WaitingScan {
            server_name: identity.server_name.clone(),
            scanner,
            identity: identity.clone(),
        };
        let peers = self.clone();
        tracing::info!(server = %identity.server_name, "对方扫了我们的码，等主人同意");
        (
            PeerReply::AwaitingApproval {
                identity: self.own_identity(),
            },
            Some(Box::pin(async move {
                peers.wait_for_answer(waiting, card).await;
            })),
        )
    }

    /// 只认自己扫过的那个 endpoint ID 发来的配对结果（ADR-0050）。
    ///
    /// 已经配上的也收：两边同意时会互相回送一条结果，出码方先配好，扫码方那条回执落到这里时它已经是
    /// `Paired`，收下才不会让扫码方白重试三次、报“配对结果没有送到”。
    pub(crate) fn on_expecting_decision(&self, from: &EndpointId) -> bool {
        self.lock().peers.values().any(|record| {
            record.endpoint_id == *from
                && matches!(record.phase, Phase::AwaitingPeer | Phase::Paired)
        })
    }

    /// 自己扫的对方把主人的决定告诉我们了。返回要执行的后续。
    pub(crate) fn on_decision(
        self: &Arc<Self>,
        from: EndpointId,
        approved: bool,
        identity: &PeerIdentity,
    ) -> Option<FollowUp> {
        let server_name = {
            let inner = self.lock();
            let record = inner
                .peers
                .values()
                .find(|record| record.phase == Phase::AwaitingPeer && record.endpoint_id == from)?;
            record.server_name.clone()
        };
        // 回执在派发处同步送回，回执一到对方就会开始探活：先把对方放进本机的入站名单再返回，
        // 对方第一次探活就能连上。
        if approved {
            self.server_allowlist.allow(from);
        }
        let peers = self.clone();
        let identity = identity.clone();
        Some(Box::pin(async move {
            if approved {
                peers.approve(&server_name, from, identity).await;
            } else {
                peers.forget_pending(&server_name);
                tracing::info!(server = %server_name, "对方的主人拒绝了配对");
            }
        }))
    }

    /// 对方删了和我们的配对：记录留着、标成“对方删了”（持久化），联邦名单撤掉。主人移除这一行才真删。
    pub(crate) fn on_removed(&self, from: EndpointId, server_name: &str) {
        let mut inner = self.lock();
        let Some(record) = inner.peers.get_mut(server_name) else {
            // 对方已经是删掉之后的状态（主人移除过，或握手被拒已经标过）：收到通知也不算错。
            return;
        };
        if record.phase != Phase::Paired || record.endpoint_id != from {
            tracing::warn!(
                server = %server_name,
                from = %from.fmt_short(),
                "删除配对的通知和记录对不上，不理"
            );
            return;
        }
        if record.phase == Phase::RemovedByPeer {
            return;
        }
        record.phase = Phase::RemovedByPeer;
        if let Err(e) = self.save(&inner.peers) {
            tracing::error!(error = %e, "配对记录写不进去");
        }
        self.retract(server_name, from);
        tracing::info!(server = %server_name, "对方删了和我们的配对，记录留着并标成对方已断开");
    }

    /// 发卡片、等回答、按回答落记录并通知对方。
    async fn wait_for_answer(
        self: &Arc<Self>,
        waiting: WaitingScan,
        card: wonton_protocol::ConfirmationCard,
    ) {
        let draft = crate::mcp::Draft {
            title: card.title,
            lines: card.lines,
            choices: card.choices,
            scope: None,
            plugin_id: None,
            peer_request: card.peer_request,
        };
        match self.confirmations.ask(&caller(), draft).await {
            Ok(ticket) => {
                self.approve(&waiting.server_name, waiting.scanner, waiting.identity)
                    .await;
                // 配对在同意时已经成立，卡片只报告结果。
                let langs = (self.languages)();
                self.confirmations
                    .finish(
                        ticket,
                        ConfirmationState::Succeeded {
                            message: crate::l10n::text(&langs, "peers-connected", &[]),
                        },
                    )
                    .await;
            }
            Err(crate::mcp::Refusal::Rejected) | Err(crate::mcp::Refusal::Expired) => {
                self.reject(&waiting.server_name, waiting.scanner, waiting.identity)
                    .await;
            }
            Err(e) => {
                // 卡片没有发出去（没接 Matrix、等回答的太多）：这条扫码作废，对方过期后重新扫。
                tracing::warn!(error = %e.message(), "服务器配对的确认卡片发不出去");
                let mut inner = self.lock();
                let matches = inner.peers.get(&waiting.server_name).is_some_and(|record| {
                    record.phase == Phase::AwaitingOwner && record.endpoint_id == waiting.scanner
                });
                if matches {
                    inner.peers.remove(&waiting.server_name);
                    if let Err(e) = self.save(&inner.peers) {
                        tracing::error!(error = %e, "配对记录写不进去");
                    }
                }
            }
        }
    }
}
