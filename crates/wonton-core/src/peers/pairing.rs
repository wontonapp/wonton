//! 配对的业务流程：收请求、扫码、同意拒绝、问结果、删配对，以及隧道上 [`PeerPairHandler`] 的实现。
//!
//! 删配对分两种：主人移除（记录真删，通知对方一次，尽力而为）和对方删了（记录留着、标成对方已断开，
//! 等主人移除）。重新配对时，标过的记录被新记录替换。
//!
//! [`PeerPairHandler`] 的方法由隧道在连接上同步调用，配对的后续（送结果、落记录）是异步的，
//! 所以真正的方法挂在 `Arc<Peers>` 上，由 [`PeerPairing`] 包装后交给隧道，后续任务克隆这个 Arc。

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use iroh::EndpointId;
use wonton_protocol::PeerError;
use wonton_tunnel::peer_pair::{
    PeerIdentity, PeerPairHandler, PeerPairToken, PeerRejectReason, PeerReply, deliver_decision,
    notify_removed,
};

use super::{
    HOUR_MS, MAX_PENDING, MAX_PER_HOUR, PeerRecord, PeerRequest, Peers, Phase, RejectedRequest,
    code, now_ms,
};
use crate::secret::random_token;

/// 配对结果重试送几次：对方暂时忙或刚好在重启时还能送到。
const DECISION_TRIES: u32 = 3;

/// 两次重试之间隔多久。
const DECISION_RETRY_PAUSE: Duration = Duration::from_secs(3);

/// 一次配对的后续任务。
pub(super) type FollowUp = Pin<Box<dyn Future<Output = ()> + Send>>;

/// 隧道用的包装：同步地应答，把异步的后续交给 tokio。挂在 iroh router 上：
/// `PeerPairProtocol::new(Arc::new(PeerPairing(peers.clone())))`。
#[derive(Debug)]
pub struct PeerPairing(pub Arc<Peers>);

impl PeerPairHandler for PeerPairing {
    fn scanned(
        &self,
        scanner: EndpointId,
        identity: &PeerIdentity,
        token: &PeerPairToken,
    ) -> PeerReply {
        let (reply, followup) = self.0.on_scanned(scanner, identity, token);
        if let Some(followup) = followup {
            tokio::spawn(followup);
        }
        reply
    }

    fn outcome(&self, scanner: EndpointId) -> PeerReply {
        self.0.on_outcome(scanner)
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
pub(super) async fn deliver_with_retries(
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
pub(super) fn reject_to_peer_error(reason: PeerRejectReason) -> PeerError {
    match reason {
        PeerRejectReason::CodeRevoked => PeerError::CodeRevoked,
        PeerRejectReason::Rejected => PeerError::PeerRejected,
        PeerRejectReason::TooManyRequests => PeerError::TooManyRequests,
        PeerRejectReason::AlreadyPaired => PeerError::AlreadyPaired,
        PeerRejectReason::SelfPairing => PeerError::SelfPairing,
        // 请求不合规：码不可信，一律按“码读不懂”说清。
        PeerRejectReason::BadRequest => PeerError::CodeInvalid,
        // 出码方自己出错、比这个核心新的原因：如实归到“其他”。
        PeerRejectReason::ServerError | PeerRejectReason::Other => PeerError::Other,
    }
}

pub(super) fn storage_failed(error: super::PeerStoreError, message: &str) -> PeerError {
    tracing::error!(error = %error, "{message}");
    PeerError::Other
}

/// 自报的用户 ID 必须在自报的服务器名下（ADR-0050）。
fn owner_matches(identity: &PeerIdentity) -> bool {
    let suffix = format!(":{}", identity.server_name);
    identity.owner_user_id.starts_with('@')
        && identity.owner_user_id.len() > suffix.len()
        && identity.owner_user_id.ends_with(&suffix)
}

impl Peers {
    /// 主人移除一台服务器：记录删掉、名单撤掉，自己发出的请求一并取消，通知对方（尽力而为，不挡着返回）。
    /// 对方删了我们时，那边已经没有记录，收到通知也只当没这回事，不算错。
    pub async fn remove(
        self: &Arc<Self>,
        server_name: &str,
    ) -> Result<bool, super::PeerStoreError> {
        let (record, cancelled) = {
            let mut inner = self.lock();
            let record = inner.peers.remove(server_name);
            let cancelled: Vec<PeerRequest> = {
                let (matching, rest): (Vec<_>, Vec<_>) = inner
                    .requests
                    .drain(..)
                    .partition(|request| request.server_name == server_name);
                inner.requests = rest;
                matching
            };
            if record.is_none() && cancelled.is_empty() {
                return Ok(false);
            }
            self.save(&inner)?;
            if let Some(record) = &record {
                self.retract(server_name, record.endpoint_id);
            }
            (record, cancelled)
        };
        tracing::info!(server = %server_name, "移除了和对方服务器的配对");
        // 对方删了我们、这边只是把记录清掉时，它的记录已经没了：通知照样发，送到也只当没这回事。
        let own = self.own.server_name.clone();
        let mut targets = Vec::new();
        match record {
            Some(record) if record.phase != Phase::RemovedByPeer => {
                targets.push(record.endpoint_addr())
            }
            Some(_) => {}
            None => {}
        }
        for request in cancelled {
            targets.push(request.endpoint_addr());
        }
        for addr in targets {
            let endpoint = self.tunnel.endpoint.clone();
            let own = own.clone();
            tokio::spawn(async move {
                // 一次就够：对方离线时，它以后在握手被拒或问结果时知道（ADR-0050 的状态表）。
                if let Err(e) = notify_removed(&endpoint, addr, &own).await {
                    tracing::info!(error = %e, "移除配对的通知没有送到对方");
                }
            });
        }
        Ok(true)
    }

    /// 名单维护：联邦名单和已配对服务器名里撤掉。
    pub(super) fn retract(&self, server_name: &str, endpoint_id: EndpointId) {
        self.server_allowlist.revoke(&endpoint_id);
        self.names
            .send_if_modified(|names| names.remove(server_name));
    }

    /// 同意一条请求：记成已配对、放进名单，把结果送给对方（对方也记成已配对），结果送出后再让转发组件开始探活。
    ///
    /// 顺序要紧：对方一收下结果就会回送一条结果并开始探活，所以要先把对方放进本机的入站名单，再送结果；
    /// 本机自己的探活由已配对名单触发，等结果送出（或确认送不到）之后再加，否则会赶在对方把本机放进
    /// 名单之前探活，第一次就被拒。
    pub async fn approve(self: &Arc<Self>, id: &str) -> Result<(), PeerError> {
        let request = {
            let mut inner = self.lock();
            let Some(index) = inner.requests.iter().position(|row| row.id == id) else {
                return Err(PeerError::RequestNotFound);
            };
            let request = inner.requests[index].clone();
            if let Some(record) = inner.peers.get(&request.server_name)
                && record.endpoint_id != request.endpoint_id
            {
                return Err(PeerError::AlreadyPaired);
            }
            inner.requests.remove(index);
            inner
                .rejected
                .retain(|row| row.endpoint_id != request.endpoint_id);
            inner.peers.insert(
                request.server_name.clone(),
                PeerRecord {
                    server_name: request.server_name.clone(),
                    endpoint_id: request.endpoint_id,
                    relays: request.relays.clone(),
                    addrs: request.addrs.clone(),
                    owner_user_id: request.owner_user_id.clone(),
                    owner_display_name: request.owner_display_name.clone(),
                    paired_at_ms: now_ms(),
                    phase: Phase::Paired,
                    last_connected_ms: None,
                },
            );
            self.server_allowlist.allow(request.endpoint_id);
            if let Err(e) = self.save(&inner) {
                tracing::error!(error = %e, "配对记录写不进去");
            }
            request
        };
        tracing::info!(server = %request.server_name, "同意了和对方服务器的配对");
        let peers = self.clone();
        let own = self.own_identity();
        let addr = request.endpoint_addr();
        let name = request.server_name.clone();
        tokio::spawn(async move {
            if !deliver_with_retries(&peers.tunnel.endpoint, addr, true, own).await {
                tracing::warn!(
                    server = %name,
                    "配对结果没有送到对方：对方来问结果时会知道"
                );
            }
            let still_paired = peers
                .lock()
                .peers
                .get(&name)
                .is_some_and(|record| record.phase == Phase::Paired);
            if still_paired {
                peers.names.send_if_modified(|names| names.insert(name));
            }
        });
        Ok(())
    }

    /// 拒绝一条请求：记进“已拒绝”（按 endpoint ID），把结果送给对方。
    pub async fn reject(self: &Arc<Self>, id: &str) -> Result<(), PeerError> {
        let request = {
            let mut inner = self.lock();
            let Some(index) = inner.requests.iter().position(|row| row.id == id) else {
                return Err(PeerError::RequestNotFound);
            };
            let request = inner.requests.remove(index);
            inner.rejected.push(RejectedRequest {
                id: request.id.clone(),
                endpoint_id: request.endpoint_id,
                server_name: request.server_name.clone(),
                owner_user_id: request.owner_user_id.clone(),
                owner_display_name: request.owner_display_name.clone(),
                rejected_at_ms: now_ms(),
            });
            if let Err(e) = self.save(&inner) {
                tracing::error!(error = %e, "配对记录写不进去");
            }
            request
        };
        tracing::info!(server = %request.server_name, "拒绝了和对方服务器的配对");
        let peers = self.clone();
        let own = self.own_identity();
        let addr = request.endpoint_addr();
        tokio::spawn(async move {
            deliver_with_retries(&peers.tunnel.endpoint, addr, false, own).await;
        });
        Ok(())
    }

    /// 撤销一次拒绝：对方再发就照常收下。
    pub async fn unreject(&self, id: &str) -> Result<(), PeerError> {
        let mut inner = self.lock();
        let before = inner.rejected.len();
        inner.rejected.retain(|row| row.id != id);
        if inner.rejected.len() == before {
            return Err(PeerError::RequestNotFound);
        }
        self.save(&inner)
            .map_err(|e| storage_failed(e, "配对记录写不进去"))
    }

    /// 有人扫了我们的码：验令牌，按限频收下并落盘，回“等对方同意”。两边互相发了请求时直接成立并送结果。
    pub(crate) fn on_scanned(
        self: &Arc<Self>,
        scanner: EndpointId,
        identity: &PeerIdentity,
        token: &PeerPairToken,
    ) -> (PeerReply, Option<FollowUp>) {
        if let Some(refusal) = self.refuse_scan(scanner, identity, token) {
            return (refusal, None);
        }
        let (fresh, mutual) = match self.accept_scan(scanner, identity) {
            Ok(accepted) => accepted,
            Err(refusal) => return (*refusal, None),
        };
        // 对方也扫过我们的码：两边都不用再点，直接把结果送过去。
        let followup: Option<FollowUp> = mutual.then(|| {
            let peers = self.clone();
            let name = identity.server_name.clone();
            let identity = identity.clone();
            Box::pin(async move {
                peers.pair_local(&name, scanner, identity);
            }) as FollowUp
        });
        if let Some(request) = fresh {
            self.notify(&request);
            tracing::info!(server = %identity.server_name, "对方扫了我们的码，等主人同意");
        }
        (
            PeerReply::AwaitingApproval {
                identity: self.own_identity(),
            },
            followup,
        )
    }

    /// 收下之前的检查：扫自己、自报的身份对不上、令牌不对、已经配对过、名字被别的 endpoint 占着、
    /// 主人拒绝过。都不拦时为空。
    fn refuse_scan(
        &self,
        scanner: EndpointId,
        identity: &PeerIdentity,
        token: &PeerPairToken,
    ) -> Option<PeerReply> {
        let error =
            |reason: PeerRejectReason, message: String| PeerReply::Error { reason, message };
        if identity.server_name == self.own.server_name || scanner == self.tunnel.endpoint.id() {
            return Some(error(
                PeerRejectReason::SelfPairing,
                "扫了自己的服务器".to_owned(),
            ));
        }
        if identity.server_name.trim().is_empty() || !owner_matches(identity) {
            return Some(error(
                PeerRejectReason::BadRequest,
                format!("报的用户 ID {} 不在服务器名下", identity.owner_user_id),
            ));
        }
        let inner = self.lock();
        if let Err(reject) = code::verify(&inner.token, token) {
            return Some(error(reject.reason, reject.message));
        }
        match inner.peers.get(&identity.server_name) {
            Some(record) if record.phase == Phase::Paired => {
                return Some(error(
                    PeerRejectReason::AlreadyPaired,
                    format!("和 {} 已经配对过", identity.server_name),
                ));
            }
            // 名字被另一个 endpoint 占着（包括“对方已断开连接”的记录）。
            Some(record) if record.endpoint_id != scanner => {
                return Some(error(
                    PeerRejectReason::AlreadyPaired,
                    format!("{} 这个名字已经被另一台服务器占用", identity.server_name),
                ));
            }
            _ => {}
        }
        inner
            .rejected
            .iter()
            .any(|row| row.endpoint_id == scanner)
            .then(|| {
                error(
                    PeerRejectReason::Rejected,
                    "主人拒绝过这台服务器，还没撤销".to_owned(),
                )
            })
    }

    /// 收下一条请求并落盘。同一台服务器再发只更新那一条，不新收、不再提醒。
    /// 返回新收的那一条（更新时为空）和“两边互相发了请求”。
    fn accept_scan(
        &self,
        scanner: EndpointId,
        identity: &PeerIdentity,
    ) -> Result<(Option<PeerRequest>, bool), Box<PeerReply>> {
        let mut inner = self.lock();
        let mutual = inner
            .peers
            .get(&identity.server_name)
            .is_some_and(|record| {
                record.phase == Phase::AwaitingPeer && record.endpoint_id == scanner
            });
        if let Some(request) = inner
            .requests
            .iter_mut()
            .find(|row| row.endpoint_id == scanner)
        {
            request.server_name = identity.server_name.clone();
            request.owner_user_id = identity.owner_user_id.clone();
            request.owner_display_name = identity.owner_display_name.clone();
            request.relays = identity.relays.clone();
            request.addrs = identity.addrs.clone();
            request.received_at_ms = now_ms();
            if let Err(e) = self.save(&inner) {
                tracing::error!(error = %e, "配对记录写不进去");
            }
            return Ok((None, mutual));
        }
        let now = now_ms();
        inner.received.retain(|at| at.saturating_add(HOUR_MS) > now);
        if inner.requests.len() >= MAX_PENDING || inner.received.len() >= MAX_PER_HOUR {
            return Err(Box::new(PeerReply::Error {
                reason: PeerRejectReason::TooManyRequests,
                message: "待处理的请求满了或者这一小时收得太多".to_owned(),
            }));
        }
        inner.received.push(now);
        let request = PeerRequest {
            id: random_token(),
            endpoint_id: scanner,
            server_name: identity.server_name.clone(),
            owner_user_id: identity.owner_user_id.clone(),
            owner_display_name: identity.owner_display_name.clone(),
            relays: identity.relays.clone(),
            addrs: identity.addrs.clone(),
            received_at_ms: now,
        };
        inner.requests.push(request.clone());
        if self.save(&inner).is_err() {
            return Err(Box::new(PeerReply::Error {
                reason: PeerRejectReason::ServerError,
                message: "服务器写配对记录失败，稍后再试".to_owned(),
            }));
        }
        Ok((Some(request), mutual))
    }

    /// 两边互相发了请求：记成已配对、放进名单，把结果送给对方。
    pub(super) fn pair_local(
        self: &Arc<Self>,
        server_name: &str,
        endpoint: EndpointId,
        identity: PeerIdentity,
    ) {
        {
            let mut inner = self.lock();
            let Some(record) = inner.peers.get_mut(server_name) else {
                return;
            };
            if record.phase == Phase::Paired {
                return;
            }
            record.phase = Phase::Paired;
            record.endpoint_id = endpoint;
            record.relays = identity.relays.clone();
            record.addrs = identity.addrs.clone();
            record.owner_user_id = identity.owner_user_id.clone();
            record.owner_display_name = identity.owner_display_name.clone();
            record.paired_at_ms = now_ms();
            // 成立以后这条请求就处理完了：不再列出，也不占待处理的名额。
            inner.requests.retain(|row| row.endpoint_id != endpoint);
            inner.rejected.retain(|row| row.endpoint_id != endpoint);
            if let Err(e) = self.save(&inner) {
                tracing::error!(error = %e, "配对记录写不进去");
                return;
            }
            // 对方一收下结果就会开始探活：先把它放进入站名单。自己的探活等结果送到再加，
            // 免得赶在对方把本机放进名单之前探活（同 `approve` 的先后）。
            self.server_allowlist.allow(endpoint);
        }
        tracing::info!(server = %server_name, "两边互相发了请求，配对直接成立");
        let peers = self.clone();
        let own = self.own_identity();
        let name = server_name.to_owned();
        let addr = super::endpoint_addr(endpoint, &identity.relays, &identity.addrs);
        tokio::spawn(async move {
            // 对方还要收下这条结果：送不到也不影响本机，对方回头来问就成立。
            deliver_with_retries(&peers.tunnel.endpoint, addr, true, own).await;
            let still_paired = peers
                .lock()
                .peers
                .get(&name)
                .is_some_and(|record| record.phase == Phase::Paired);
            if still_paired {
                peers.names.send_if_modified(|names| names.insert(name));
            }
        });
    }

    /// 对方删了和我们的配对、或者取消了自己发出的请求：删掉它发来的请求；
    /// 配对的记录留着、标成“对方删了”（持久化），联邦名单撤掉。主人移除这一行才真删。
    pub(crate) fn on_removed(&self, from: EndpointId, server_name: &str) {
        let mut inner = self.lock();
        let before = inner.requests.len();
        inner.requests.retain(|request| request.endpoint_id != from);
        if inner.requests.len() != before {
            if let Err(e) = self.save(&inner) {
                tracing::error!(error = %e, "配对记录写不进去");
            }
            tracing::info!(server = %server_name, "对方取消了配对请求");
            return;
        }
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
        record.phase = Phase::RemovedByPeer;
        if let Err(e) = self.save(&inner) {
            tracing::error!(error = %e, "配对记录写不进去");
        }
        self.retract(server_name, from);
        tracing::info!(server = %server_name, "对方删了和我们的配对，记录留着并标成对方已断开");
    }
}
