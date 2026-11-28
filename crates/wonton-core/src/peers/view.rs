//! 主人看到的列表：服务器行、请求、拒绝过的请求，以及新请求的私聊提醒。

use std::sync::Arc;

use iroh::EndpointId;
use wonton_protocol::{PeerRequestRow, PeerRow, PeerStatus, PeersView, RejectedPeerRow};
use wonton_tunnel::federation::PeerStatus as FederationStatus;

use super::record::{PeerRecord, PeerRequest, Phase};
use super::{Inner, Peers};
use crate::plugin::languages::Languages;

impl Peers {
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
    pub(super) fn forwarded_status(&self, server_name: &str) -> PeerStatus {
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

    pub(super) fn last_error(&self, server_name: &str) -> Option<String> {
        let guard = self.forwarder.lock().expect("转发器状态锁中毒");
        let states = guard.as_ref()?;
        match states.borrow().get(server_name)? {
            FederationStatus::Offline { last_error, .. } => Some(last_error.to_string()),
            _ => None,
        }
    }

    /// 在“Wonton”私聊里提醒主人有一条新请求，带跳到“其他服务器”的链接。
    pub(super) fn notify(self: &Arc<Self>, request: &PeerRequest) {
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
