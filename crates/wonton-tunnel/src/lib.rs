//! iroh 1.x 隧道，服务端与客户端共用。
//!
//! 服务端只接受已配对设备的 endpoint ID；客户端在本机提供 HTTP 代理，把 Matrix 请求和小程序流量
//! 转发到家里服务器的核心网关。设计见 `docs/adr/0004-iroh-tunnel.md`。
//!
//! - 服务端：[`TunnelServer`]（独立运行）或 [`TunnelProtocol`]（挂到已有的 iroh router 上），按
//!   [`Allowlist`] 授权。每条流转发到本机的上游 HTTP 服务，或者交给嵌入方：核心从 [`TunnelListener`]
//!   取出 [`PeerStream`]，在进程内服务，并知道流来自哪台已配对设备。
//! - 联邦转发：[`federation`] 模块把本机 Tuwunel 的 Matrix 联邦流量经 iroh 转给配对过的服务器
//!   （ADR-0050），ALPN 见 [`federation::FEDERATION_ALPN`]。
//! - 客户端：[`TunnelClient`] 复用一条连接、断开后按需重连，并报告 [`TunnelStatus`]；
//!   [`LocalProxy`] 在回环地址上监听，把本机 HTTP 客户端的连接经隧道转发过去。App 用带会话凭据
//!   （[`ProxyCredential`]）的 HTTP 正向代理，同机的其他 App 借用不了本设备的配对身份。
//! - 线协议见 `wire` 模块：一条 QUIC 连接上每条双向流对应一条 TCP 连接，做法参考 dumbpipe。
//! - 插件端口：[`TunnelClient::open_port`] 开 TCP 端口流，[`TunnelClient::open_udp`] 开 UDP 流向，载荷走
//!   同一连接上的 QUIC 数据报（ADR-0072）。
//! - 扫码配对：二维码里的配对码和配对的 ALPN 见 [`pair`] 模块。
//! - 服务器配对：两台服务器的主人互相配对服务器（ADR-0050），码和 ALPN 见 [`peer_pair`] 模块。

mod allowlist;
mod blocking;
mod client;
mod credential;
mod embed;
mod error;
pub mod federation;
mod forward;
mod http_front;
mod pacing;
pub mod pair;
pub mod peer_pair;
mod ports;
mod proxy;
pub mod relays;
mod server;
#[cfg(feature = "test-hooks")]
#[doc(hidden)]
pub mod test_server;
#[cfg(feature = "test-hooks")]
#[doc(hidden)]
pub mod test_support;
mod udp;
pub mod wire;

pub use allowlist::Allowlist;
pub use blocking::off_workers;
pub use client::{
    PathInfo, PathKind, SuspendClock, TunnelClient, TunnelStats, TunnelStatus, TunnelStreamStats,
};
pub use credential::{ProxyAccess, ProxyCredential};
pub use embed::{PeerStream, TunnelListener};
pub use error::TunnelError;
pub use ports::{PortResolveError, PortResolver};
pub use proxy::{ERROR_HEADER, LocalProxy};
pub use server::{TunnelProtocol, TunnelServer};
pub use udp::{PortError, PortStream, UdpFlow, UdpReceiver, UdpSender};
pub use wire::ALPN;
