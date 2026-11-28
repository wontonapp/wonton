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

mod code;
mod federation;
pub mod pairing;
mod pin;
mod record;
pub use pin::{OwnCertificate, PeerPin, PinState, load_own_certificate, window_expired};
#[cfg(test)]
mod peers_tests;
mod scanning;
mod store;
mod view;

pub(crate) use record::{HOUR_MS, MAX_PENDING, MAX_PER_HOUR, endpoint_addr, now_ms};
pub use record::{
    OwnIdentity, OwnerNotices, PeerRecord, PeerRequest, PeerStoreError, Phase, RejectedRequest,
};

pub(crate) use store::Inner;
pub use store::{Peers, Stored};
