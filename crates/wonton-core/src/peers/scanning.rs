//! 扫码方和问结果：主人的设备扫来一个码，记成“等对方同意”，启动时和之后每十分钟问一次结果，
//! 也收对方的推送。被拒绝标成“对方拒绝了”。

use std::sync::Arc;
use std::time::Duration;

use iroh::EndpointId;
use wonton_protocol::PeerError;
use wonton_tunnel::peer_pair::{PeerIdentity, PeerRejectReason, PeerReply, ask_outcome};

use super::pairing::{FollowUp, reject_to_peer_error, storage_failed};
use super::{OUTCOME_POLL, PeerRecord, Peers, Phase};

/// 扫码之后第一次问结果之前等多久：扫码的回应刚才已经说过“等对方同意”，马上再问一遍没有新信息；
/// 太早的话核心刚启动、配对的协议还没挂上。
const OUTCOME_FIRST: Duration = Duration::from_secs(5);

impl Peers {
    /// 主人的设备扫来一个码：连出码方，记成“等对方同意”，然后自己问结果。
    pub async fn scan(self: &Arc<Self>, code_text: &str) -> Result<(), PeerError> {
        let code = wonton_tunnel::peer_pair::PeerPairingCode::parse(code_text).map_err(|e| {
            // 码在发出连接之前就读不懂：不打日志的话，两边的日志里都看不到这次扫码。
            tracing::info!(error = %e, "扫来的码读不懂");
            PeerError::CodeInvalid
        })?;
        let addr = code.server_addr();
        self.scan_to(&code, addr, self.own_identity()).await
    }

    /// 和 [`Self::scan`] 一样，但直接连已知地址：码里只有中继，同机直连或拿到过对方地址时用它。
    pub async fn scan_to(
        self: &Arc<Self>,
        code: &wonton_tunnel::peer_pair::PeerPairingCode,
        addr: iroh::EndpointAddr,
        identity: PeerIdentity,
    ) -> Result<(), PeerError> {
        // 扫了自己：不用连，直接说清。
        if code.server_name == self.own.server_name || code.server_id == self.tunnel.endpoint.id() {
            return Err(PeerError::SelfPairing);
        }
        {
            let mut inner = self.lock();
            match inner.peers.get(&code.server_name) {
                // 同一个 endpoint 已经配上了。
                Some(record)
                    if record.endpoint_id == code.server_id && record.phase == Phase::Paired =>
                {
                    return Err(PeerError::AlreadyPaired);
                }
                // 名字被另一个 endpoint 占着（包括“对方已断开连接”的记录）。
                Some(record) if record.endpoint_id != code.server_id => {
                    return Err(PeerError::AlreadyPaired);
                }
                _ => {}
            }
            // 主人自己去扫对方的码：拒绝记录清掉，对方再发就照常收下。
            inner
                .rejected
                .retain(|row| row.endpoint_id != code.server_id);
            let record = PeerRecord {
                server_name: code.server_name.clone(),
                endpoint_id: code.server_id,
                relays: code.relay_urls.iter().map(|url| url.to_string()).collect(),
                addrs: Vec::new(),
                owner_user_id: String::new(),
                owner_display_name: String::new(),
                paired_at_ms: 0,
                phase: Phase::AwaitingPeer,
                last_connected_ms: None,
            };
            inner.peers.insert(record.server_name.clone(), record);
            self.save(&inner)
                .map_err(|e| storage_failed(e, "扫码的记录写不进去"))?;
        }
        let server_name = code.server_name.clone();
        let token = code.token.clone();
        match wonton_tunnel::peer_pair::scan_to(&self.tunnel.endpoint, addr, token, identity).await
        {
            Ok(PeerReply::AwaitingApproval { identity }) => {
                self.note_awaiting(&server_name, &identity)?;
                tracing::info!(server = %server_name, "扫了对方的服务器，等对方同意");
                self.spawn_poll(&server_name);
                Ok(())
            }
            // 对方也扫了我们的码：请求直接成立。
            Ok(PeerReply::Approved { identity }) => {
                if identity.server_name != server_name {
                    self.forget_pending(&server_name);
                    return Err(PeerError::CodeInvalid);
                }
                let bound = {
                    let inner = self.lock();
                    inner
                        .peers
                        .get(&server_name)
                        .map(|record| record.endpoint_id)
                };
                match bound {
                    Some(endpoint) => {
                        self.server_allowlist.allow(endpoint);
                        self.pair_local(&server_name, endpoint, identity);
                        Ok(())
                    }
                    None => Err(PeerError::PeerUnreachable),
                }
            }
            Ok(PeerReply::Declined) => {
                self.mark_declined(&server_name, code.server_id);
                Err(PeerError::PeerRejected)
            }
            Ok(PeerReply::Error { reason, .. }) => {
                if reason == PeerRejectReason::Rejected {
                    self.mark_declined(&server_name, code.server_id);
                } else {
                    self.forget_pending(&server_name);
                }
                Err(reject_to_peer_error(reason))
            }
            Ok(other) => {
                self.forget_pending(&server_name);
                tracing::warn!(reply = ?other, "扫码的回应不对");
                Err(PeerError::CodeInvalid)
            }
            Err(e) => {
                self.forget_pending(&server_name);
                tracing::info!(error = %e, "连不上出码方");
                Err(PeerError::PeerUnreachable)
            }
        }
    }

    /// 把对方的身份记到还在等的记录上；对方报的服务器名和码里对不上时扔掉记录。
    fn note_awaiting(&self, server_name: &str, identity: &PeerIdentity) -> Result<(), PeerError> {
        if identity.server_name != server_name {
            self.forget_pending(server_name);
            return Err(PeerError::CodeInvalid);
        }
        let mut inner = self.lock();
        let Some(record) = inner.peers.get_mut(server_name) else {
            // 记录在等待对方回应时被取消或换掉了：当作没有扫过。
            return Err(PeerError::PeerUnreachable);
        };
        record.owner_user_id = identity.owner_user_id.clone();
        record.owner_display_name = identity.owner_display_name.clone();
        record.relays = identity.relays.clone();
        record.addrs = identity.addrs.clone();
        self.save(&inner)
            .map_err(|e| storage_failed(e, "扫码的记录写不进去"))
    }

    /// 删掉还在等结果的扫码记录（对方拒绝了、连不上）。
    pub(crate) fn forget_pending(&self, server_name: &str) {
        let mut inner = self.lock();
        let pending = inner
            .peers
            .get(server_name)
            .is_some_and(|record| record.phase == Phase::AwaitingPeer);
        if pending {
            inner.peers.remove(server_name);
            if let Err(e) = self.save(&inner) {
                tracing::error!(error = %e, "配对记录写不进去");
            }
        }
    }

    /// 对方拒绝了：记录留着，显示“对方拒绝了”，等主人移除。
    fn mark_declined(&self, server_name: &str, endpoint: EndpointId) {
        let mut inner = self.lock();
        let Some(record) = inner.peers.get_mut(server_name) else {
            return;
        };
        if record.endpoint_id != endpoint || record.phase == Phase::Paired {
            return;
        }
        record.phase = Phase::Declined;
        if let Err(e) = self.save(&inner) {
            tracing::error!(error = %e, "配对记录写不进去");
        }
    }

    /// 问结果：这台服务器对这个 endpoint ID 有没有请求，进展到哪一步。
    pub(crate) fn on_outcome(&self, scanner: EndpointId) -> PeerReply {
        let inner = self.lock();
        if inner
            .peers
            .values()
            .any(|record| record.endpoint_id == scanner && record.phase == Phase::Paired)
        {
            return PeerReply::Approved {
                identity: self.own_identity(),
            };
        }
        if inner.requests.iter().any(|row| row.endpoint_id == scanner) {
            return PeerReply::Pending;
        }
        if inner.rejected.iter().any(|row| row.endpoint_id == scanner) {
            return PeerReply::Declined;
        }
        PeerReply::UnknownRequest
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
        if identity.server_name != server_name {
            tracing::warn!(
                from = %from.fmt_short(),
                "配对结果报的服务器名和码里的对不上，不理"
            );
            return None;
        }
        // 回执在派发处同步送回，回执一到对方就会开始探活：先把对方放进本机的入站名单再返回，
        // 对方第一次探活就能连上。
        if approved {
            self.server_allowlist.allow(from);
        }
        let peers = self.clone();
        let identity = identity.clone();
        Some(Box::pin(async move {
            if approved {
                peers.pair_local(&server_name, from, identity);
            } else {
                peers.mark_declined(&server_name, from);
            }
        }))
    }
}

impl Peers {
    /// 起一个问结果的后台任务：先等几秒，之后每十分钟一次，记录不再是“等对方同意”就停。
    pub(super) fn spawn_poll(self: &Arc<Self>, server_name: &str) {
        let peers = Arc::downgrade(self);
        let name = server_name.to_owned();
        tokio::spawn(async move {
            tokio::time::sleep(OUTCOME_FIRST).await;
            // 问到了“还在等”才按 10 分钟一次；没问到（连不上、回应看不懂）从 OUTCOME_FIRST 起翻倍重问，
            // 封顶 OUTCOME_POLL。只问一次就等 10 分钟的话，刚启动或网络抖一下，“等对方同意”就多挂 10 分钟。
            let mut retry = OUTCOME_FIRST;
            loop {
                let Some(link) = peers.upgrade() else {
                    return;
                };
                let answered = link.check_outcome(&name).await;
                if !link.is_awaiting(&name) {
                    return;
                }
                drop(link);
                let pause = if answered {
                    retry = OUTCOME_FIRST;
                    OUTCOME_POLL
                } else {
                    let pause = retry;
                    retry = (retry * 2).min(OUTCOME_POLL);
                    pause
                };
                tokio::time::sleep(pause).await;
            }
        });
    }

    fn is_awaiting(&self, server_name: &str) -> bool {
        self.lock()
            .peers
            .get(server_name)
            .is_some_and(|record| record.phase == Phase::AwaitingPeer)
    }

    /// 问一次结果：对方同意就成立，拒绝就标成拒绝，没答上来下个周期再问。对方给了看得懂的回应
    /// （包括“还在等”）时返回 true；连不上、回应看不懂时返回 false，由调用方很快再问。
    async fn check_outcome(self: &Arc<Self>, server_name: &str) -> bool {
        let target = {
            let inner = self.lock();
            inner
                .peers
                .get(server_name)
                .filter(|record| record.phase == Phase::AwaitingPeer)
                .map(|record| (record.endpoint_id, record.endpoint_addr()))
        };
        let Some((endpoint, addr)) = target else {
            return true;
        };
        match ask_outcome(&self.tunnel.endpoint, addr).await {
            Ok(PeerReply::Approved { identity }) => {
                if identity.server_name != server_name {
                    tracing::warn!(
                        server = %server_name,
                        "问结果的回应报的服务器名对不上，不理"
                    );
                    return true;
                }
                // 对方在等本机的探活：先放行再落记录。
                self.server_allowlist.allow(endpoint);
                self.pair_local(server_name, endpoint, identity);
                true
            }
            Ok(PeerReply::Declined) => {
                self.mark_declined(server_name, endpoint);
                true
            }
            Ok(PeerReply::Pending | PeerReply::UnknownRequest) => true,
            Ok(other) => {
                tracing::debug!(reply = ?other, "问结果的回应看不懂，稍后再问");
                false
            }
            Err(e) => {
                tracing::debug!(error = %e, "没有问到配对结果，稍后再问");
                false
            }
        }
    }

    /// 测试用：马上问一次结果（不等第一个周期）。
    #[cfg(test)]
    pub(crate) async fn poll_outcome(self: &Arc<Self>, server_name: &str) {
        self.check_outcome(server_name).await;
    }
}
