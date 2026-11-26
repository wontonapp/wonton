//! 当前连接正在使用的传输路径：壳和连接质量判定都只读这个快照，不碰 iroh 的连接内部。
//!
//! 代理的 TUN 把对端地址伪造成 198.18.0.0/15（fake-IP）。这样得到的“直连”其实经代理出口绕了一圈，
//! iroh 自己分不出来，照样当正常直连挑；[`PreferRealPaths`] 把这档路径排到真直连和中继后面。

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use iroh::endpoint::transports::{
    Addr, FourTuple, PathSelection, PathSelectionContext, PathSelector,
};
use iroh::{EndpointAddr, TransportAddr};

/// 代理 TUN 的假地址段：198.18.0.0/15，按人话就是 198.18 和 198.19 两个 B 类段。
const FAKE_IP_NETWORK: u32 = u32::from_be_bytes([198, 18, 0, 0]);
const FAKE_IP_PREFIX: u8 = 15;

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
    /// 直连的对端是局域网地址。经代理 TUN 的假地址也算：对端就在身边，只是绕了代理。
    pub fn is_lan(&self) -> bool {
        let TransportAddr::Ip(addr) = &self.remote else {
            return false;
        };
        is_fake_ip(addr.ip())
            || match addr.ip() {
                IpAddr::V4(ip) => ip.is_private() || ip.is_link_local() || ip.is_loopback(),
                IpAddr::V6(ip) => {
                    ip.is_unique_local() || ip.is_unicast_link_local() || ip.is_loopback()
                }
            }
    }
}

/// 地址落在代理 TUN 的假地址段（198.18.0.0/15）里。IPv4-mapped 的 IPv6 地址按 IPv4 看。
pub fn is_fake_ip(addr: IpAddr) -> bool {
    let IpAddr::V4(ip) = addr.to_canonical() else {
        return false;
    };
    // 掩码按前缀算。32 - 0 是 32，u32 的移位 32 会溢出，/0 单独成全 0。
    let mask = if FAKE_IP_PREFIX == 0 {
        0
    } else {
        u32::MAX << (32 - FAKE_IP_PREFIX)
    };
    u32::from(ip) & mask == FAKE_IP_NETWORK & mask
}

/// 直连候选里有真地址时去掉代理 TUN 的假地址；只有假地址时原样保留，它是唯一的直连。
///
/// iroh 把首包发给全部候选，先回包的那条定成连接的第一条路径，之后只补回中继，别的直连候选不再试；选路器
/// 也只在已经打开的路径里挑。经代理绕行的假地址一旦先回包，连接就一直绕着走（`tests/tunnel/fake_ip.rs`）。
pub fn without_fake_ips(server: EndpointAddr) -> EndpointAddr {
    let fake = |addr: &TransportAddr| matches!(addr, TransportAddr::Ip(ip) if is_fake_ip(ip.ip()));
    let real = |addr: &TransportAddr| matches!(addr, TransportAddr::Ip(_)) && !fake(addr);
    if !server.addrs.iter().any(real) {
        return server;
    }
    EndpointAddr {
        id: server.id,
        addrs: server
            .addrs
            .into_iter()
            .filter(|addr| !fake(addr))
            .collect(),
    }
}

/// 选路时的优先级，越小越先。假地址排在中继后面：只有别的路径都没有时才用它。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Tier {
    Real,
    Relay,
    Fake,
}

/// iroh 默认的选路器对 IPv6 给 3 ms 的优先，这里保持一致。
const IPV6_RTT_ADVANTAGE: Duration = Duration::from_millis(3);
/// 同一档内换路要求的最低 RTT 优势，避免抖动时来回换（同 iroh 默认值）。
const RTT_SWITCHING_MIN: Duration = Duration::from_millis(5);

/// 一个候选路径。`remote` 用 iroh 的 [`Addr`] 是为了和 `PathSelectionContext::current()` 直接比较。
struct Candidate {
    remote: Addr,
    rtt: Duration,
}

impl Candidate {
    /// 越小越优先：先比档，再比去掉 IPv6 优势的 RTT。
    fn key(&self) -> (Tier, i128) {
        let tier = match &self.remote {
            Addr::Relay(..) => Tier::Relay,
            Addr::Ip(addr) if is_fake_ip(addr.ip()) => Tier::Fake,
            _ => Tier::Real,
        };
        let advantage = match (&self.remote, tier) {
            (Addr::Ip(SocketAddr::V6(_)), Tier::Real) => IPV6_RTT_ADVANTAGE,
            _ => Duration::ZERO,
        };
        let rtt = i128::try_from(self.rtt.as_nanos()).unwrap_or(i128::MAX);
        let advantage = i128::try_from(advantage.as_nanos()).unwrap_or(0);
        (tier, rtt - advantage)
    }
}

/// 从候选里挑一条要切的，返回下标；`None` 表示保持现在的路径。规则照 iroh 默认选路器：跨档立即换，
/// 同档要有 5 ms 以上的优势才换；现在的路径在候选里出现多次时按最低 RTT 的那条算。
fn choose(current: Option<&FourTuple>, candidates: &[Option<Candidate>]) -> Option<usize> {
    let current = current.map(FourTuple::remote);
    let mut best: Option<(usize, (Tier, i128))> = None;
    let mut current_key: Option<(Tier, i128)> = None;
    for (index, candidate) in candidates.iter().enumerate() {
        let Some(candidate) = candidate else {
            continue;
        };
        let key = candidate.key();
        if current.as_ref() == Some(&candidate.remote) && current_key.is_none_or(|c| key < c) {
            current_key = Some(key);
        }
        if best.as_ref().is_none_or(|(_, b)| key < *b) {
            best = Some((index, key));
        }
    }
    let (index, (best_tier, best_biased)) = best?;
    let Some((current_tier, current_biased)) = current_key else {
        return Some(index);
    };
    if current_tier != best_tier {
        return Some(index);
    }
    let min = i128::try_from(RTT_SWITCHING_MIN.as_nanos()).unwrap_or(i128::MAX);
    (best_biased + min <= current_biased).then_some(index)
}

/// 优先走真路径的选路器：真直连 > 中继 > 经代理 TUN 的假地址。
#[derive(Debug, Default)]
pub struct PreferRealPaths;

impl PathSelector for PreferRealPaths {
    fn select(&self, ctx: &PathSelectionContext<'_>) -> PathSelection {
        let paths: Vec<_> = ctx.paths().collect();
        let candidates: Vec<_> = paths
            .iter()
            .map(|path| {
                Some(Candidate {
                    remote: path.network_path().remote(),
                    rtt: path.stats()?.rtt,
                })
            })
            .collect();
        let mut selection = PathSelection::none();
        if let Some(index) = choose(ctx.current(), &candidates) {
            selection.set(&paths[index]);
        }
        selection
    }
}

/// 开关打开时给出排除假地址的选路器，关着给 `None`：endpoint 不装选路器，iroh 的默认选路一点不动。
pub fn path_selector(enabled: bool) -> Option<Arc<dyn PathSelector>> {
    enabled.then(|| Arc::new(PreferRealPaths) as Arc<dyn PathSelector>)
}

#[cfg(test)]
#[path = "path_tests.rs"]
mod path_tests;
