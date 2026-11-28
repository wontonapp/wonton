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
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use tokio::sync::watch;
use wonton_tunnel::Allowlist;
use wonton_tunnel::federation::PeerStates;

use crate::plugin::languages::Languages;
use crate::tunnel::Tunnel;

mod code;
mod forwarder;
mod identity;
pub use identity::OwnIdentity;
mod notices;
pub use notices::OwnerNotices;
pub mod pairing;
#[cfg(test)]
mod peers_tests;
mod pin;
use pin::prune_expired_pins;
pub use pin::{OwnCertificate, PeerPin, PinState, load_own_certificate, window_expired};
mod records;
pub(in crate::peers) use records::endpoint_addr;
pub use records::{PeerRecord, PeerRequest, Phase, RejectedRequest};
mod scanning;
mod store;
pub use store::{PeerStoreError, Stored};
mod view;

/// 一小时的毫秒数。
const HOUR_MS: u64 = 60 * 60 * 1000;

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

pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or_default()
}
