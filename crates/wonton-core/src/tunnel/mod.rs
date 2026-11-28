//! 隧道入口：核心内置 iroh 隧道服务端（ADR-0004），在进程内用网关服务每条隧道流。
//!
//! 每条流对应设备本地代理收到的一条 TCP 连接。核心把 QUIC 握手认证过的设备 endpoint ID 作为
//! [`TunnelPeer`] 放进请求扩展，网关据此认出主人的设备。只有这里会放入 `TunnelPeer`：局域网的 TCP
//! 请求里没有它，客户端也没法用请求头伪造。

mod addresses;
mod endpoint;
mod handle;
mod ports;
mod serve;

pub use addresses::spawn_announcing;
pub use endpoint::{bind, load_or_create_key};
pub use handle::{Tunnel, TunnelEntry};
pub use ports::PluginPortResolver;

// 只给模块文档里 [`TunnelPeer`] 的链接用。
#[allow(unused_imports)]
use crate::gateway::TunnelPeer;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
