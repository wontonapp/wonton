//! 当前连接正在使用的传输路径：壳和连接质量判定都只读这个快照，不碰 iroh 的连接内部。

use std::net::IpAddr;
use std::time::Duration;

use iroh::TransportAddr;

/// 传输路径的类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathKind {
    /// 局域网直连或打洞成功的 P2P 直连。
    Direct,
    /// 经中继服务器转发。
    Relay,
}

/// 当前连接正在使用的传输路径。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathInfo {
    /// 本次 QUIC 连接的标识，用来区分同一地址上的重连。
    pub connection_id: usize,
    /// 直连还是中继。
    pub kind: PathKind,
    /// 对端在这条路径上的地址：直连时是 IP 和端口，中继时是中继服务器。
    pub remote: TransportAddr,
    /// QUIC 对这条路径的往返时间估计。
    pub rtt: Duration,
    /// 这条路径发出的 QUIC 包数，连接质量的判定用（ADR-0032）。
    pub sent_packets: u64,
    /// 这条路径判定丢失的 QUIC 包数。
    pub lost_packets: u64,
    /// 这条路径发出的 QUIC 字节数；只为 ACK 的包记 0。
    pub sent_bytes: u64,
    /// 这条路径判定丢失的 QUIC 字节数。
    pub lost_bytes: u64,
}

impl PathInfo {
    /// 直连的对端是局域网地址。
    pub fn is_lan(&self) -> bool {
        let TransportAddr::Ip(addr) = &self.remote else {
            return false;
        };
        match addr.ip() {
            IpAddr::V4(ip) => ip.is_private() || ip.is_link_local() || ip.is_loopback(),
            IpAddr::V6(ip) => {
                ip.is_unique_local() || ip.is_unicast_link_local() || ip.is_loopback()
            }
        }
    }
}
