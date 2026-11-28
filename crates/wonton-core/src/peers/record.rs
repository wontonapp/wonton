//! 配对记录的形状：状态、记录、请求和拒绝，以及握手时发给对方的身份。

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use futures_util::future::BoxFuture;
use iroh::{EndpointAddr, EndpointId, RelayUrl};
use serde::{Deserialize, Serialize};
use wonton_tunnel::peer_pair::PeerIdentity;
use wonton_tunnel::relays::canonical_url;

use super::pin::{PeerPin, PinState};
use crate::tunnel::Tunnel;

/// 待处理的请求最多几条（ADR-0050：超出回“对方暂时收不了请求”）。
pub(crate) const MAX_PENDING: usize = 20;

/// 一小时最多收几条新请求（ADR-0050）。
pub(crate) const MAX_PER_HOUR: usize = 10;

/// 一小时的毫秒数。
pub(crate) const HOUR_MS: u64 = 60 * 60 * 1000;

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
    pub(super) fn endpoint_addr(&self) -> EndpointAddr {
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

pub(crate) fn endpoint_addr(
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
                .map(|relay| wonton_tunnel::relays::relay_url_text(&relay.url))
                .collect(),
            addrs: tunnel.direct_addrs(),
        }
    }
}

/// 在“Wonton”私聊里提醒主人（生产里是 `matrix::NoticeRoom`）。发不出去只记日志，不影响配对。
pub trait OwnerNotices: Send + Sync + 'static {
    /// 发一条消息，内容是 Matrix 的消息体（`m.text`）。
    fn send(&self, content: serde_json::Value) -> BoxFuture<'_, Result<(), String>>;
}

impl OwnerNotices for crate::matrix::NoticeRoom {
    fn send(&self, content: serde_json::Value) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async move {
            self.send(&content)
                .await
                .map(|_| ())
                .map_err(|e| e.to_string())
        })
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

pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or_default()
}
