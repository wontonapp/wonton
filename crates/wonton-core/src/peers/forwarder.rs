//! 接上联邦转发组件和公网访问总开关，跟着它们的状态更新记录。

use std::sync::Arc;

use tokio::sync::watch;
use wonton_tunnel::federation::{PeerStates, PeerStatus as FederationStatus};

use super::{Peers, Phase, now_ms};

impl Peers {
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
    pub(in crate::peers) fn note_removed_by_peer(&self, server_name: &str) {
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
