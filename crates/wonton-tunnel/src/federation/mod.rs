//! 联邦转发（ADR-0050）：两台配对过的 Wonton 服务器的 Matrix 联邦流量经 iroh 互转。
//!
//! Tuwunel 的联邦写死了 HTTPS：本机 Tuwunel 把发往对方服务器名的连接交给本模块监听的地址，模块从
//! TLS ClientHello 里读出 SNI（不终结 TLS），按配对时记下的“服务器名 → endpoint ID”绑定经 iroh 转过去；
//! 反方向上，对方经 [`FEDERATION_ALPN`] 进来的每条流原样接到本机 Tuwunel 的联邦端口。TLS 在两台
//! Tuwunel 之间端到端，本模块只按 SNI 选路，看不到联邦内容。
//!
//! - 入站：[`FederationProtocol`]，只接受服务器名单里的 endpoint ID。
//! - 出站：[`FederationForwarder`]，每台对方服务器一条共享连接，断开后按退避重连、定时探活。
//! - 状态：每台对方服务器一个 [`PeerStatus`]，经 [`FederationForwarder::watch_status`] 给嵌入方；
//!   Tuwunel 分不清对方离线和拒绝本机，由嵌入方按这里的状态区分（ADR-0050 的“状态”）。

mod outbound;
mod peer;
mod server;

pub use outbound::{FederationForwarder, OutboundErrors, ServerLookup};
pub use peer::{PeerStates, PeerStatus};
pub use server::FederationProtocol;

/// 联邦协议的 ALPN，和隧道（`wonton/tunnel/0`）、配对（`wonton/pair/0`）挂在同一个 endpoint 上：
/// 对外只有一个 UDP 端口、一个 endpoint ID（ADR-0048）。协议不兼容地变化时升级末尾的版本号。
pub const FEDERATION_ALPN: &[u8] = b"wonton/federation/0";
