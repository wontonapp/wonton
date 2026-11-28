//! 主人看到的服务器列表、请求和拒绝过的行，以及每行的状态和“详情”。

use iroh::EndpointId;
use wonton_protocol::{PeerRequestRow, PeerRow, PeerStatus, PeersView, RejectedPeerRow};
use wonton_tunnel::federation::PeerStatus as FederationStatus;

use super::{Inner, PeerRecord, PeerRequest, Peers, Phase};
use crate::plugin::languages::Languages;

impl Peers {
    /// 一台服务器在联系人看来的样子（ADR-0050）：已配对时是转发组件给的状态，没配对或配对已经移除时为空。
    pub fn contact_reach(&self, server_name: &str) -> Option<PeerStatus> {
        let paired = self
            .lock()
            .peers
            .get(server_name)
            .is_some_and(|record| record.phase == Phase::Paired);
        paired.then(|| self.forwarded_status(server_name))
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
