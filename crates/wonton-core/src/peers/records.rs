//! 配对记录和请求：落盘的形状，以及回连地址的换算。

use std::net::SocketAddr;

use iroh::{EndpointAddr, EndpointId, RelayUrl};
use serde::{Deserialize, Serialize};
use wonton_tunnel::relays::canonical_url;

use super::pin::{PeerPin, PinState};

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
    /// 钉住的对方证书（ADR-0112）：旧的和新的一起收，最新的在最后。
    #[serde(default)]
    pub peer_pin: Vec<PeerPin>,
    /// 证书的钉扎状态和待钉窗口的起点。
    #[serde(default)]
    pub pin_state: PinState,
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
    pub(in crate::peers) fn endpoint_addr(&self) -> EndpointAddr {
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

pub(in crate::peers) fn endpoint_addr(
    id: EndpointId,
    relays: &[String],
    addrs: &[SocketAddr],
) -> EndpointAddr {
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
