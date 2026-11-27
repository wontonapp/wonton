//! 连接一直走中继时，趁空闲重拨一次，让直连再有一次机会（ADR-0004“一直走中继”）。
//!
//! iroh 只在还没选定路径时把首包发给全部候选，先回包的那条成了连接的第一条路径；之后只补回中继，连接信息里别的
//! 直连候选不再试，选路器也只在已经打开的路径里挑。中继先回包的那一轮（直连头几个包丢了、直连比中继慢），连接就
//! 一直留在中继上，除非 QNT 打洞成功。上游的修法没合，这里不改 iroh，只在连接层重拨。
//!
//! 选定的路径记在 iroh 里按对端存的状态上，对端还有连接时不清：旧连接没关掉就拨，新握手的包只发往旧的中继路径，
//! 重拨白费。所以先关旧连接、等 iroh 收到关闭，再拨。旧连接上的流因此会断，只在空闲时做（见 [`StickyRelay`]）。

use std::{
    collections::BTreeSet,
    net::SocketAddr,
    time::{Duration, Instant},
};

use iroh::{EndpointAddr, endpoint::Connection};
use tracing::info;

use super::{
    Attempt, CLOSE_NORMAL, TunnelClient, TunnelError, TunnelStatus, lock, path::is_fake_ip, suspend,
};

/// 连接一直走中继这么久才重拨。iroh 每分钟打一次洞（`UPGRADE_INTERVAL`），十分钟已经试过十轮，自己换不过去了；
/// 重拨要断开旧连接上空闲的流、新请求等一次握手，不值得为短暂落在中继上的连接做。
pub(super) const RELAY_ONLY_BEFORE_REDIAL: Duration = Duration::from_secs(10 * 60);
/// 重拨过还在中继上：至少隔这么久才再重拨。直连这次没成，多半是网络本来就不让直连（对称 NAT、防火墙），
/// 频繁重拨只是反复打断。
pub(super) const REDIAL_BACKOFF: Duration = Duration::from_secs(30 * 60);
/// 一段中继期里最多重拨几次。换到直连或者连接信息里来了新的直连候选，才重新计数。
const MAX_REDIALS: u32 = 2;
/// 两次检查之间连接平均每秒收发不超过这么多字节，才算空闲：iroh 的心跳和定时检查的往返每秒只有几百字节，
/// 语音通话（单向 16 kbps 起）、下载和上传都远超过它。
const QUIET_BYTES_PER_SEC: u64 = 2 * 1024;
/// 关掉旧连接之后等这么久再拨。iroh 的对端状态机先处理信箱里的消息、后处理连接关闭（`biased` 的 select），
/// 立刻拨的话新握手的首包可能赶在“清掉选定路径”之前；服务端也要等关闭帧经中继到了才清它那一侧。网络命名空间里
/// （中继单程加 20 毫秒）不等也 5 次都换到了直连，这一秒是给真实中继的延迟留的，没有实测。不关旧连接就拨的
/// 对照 5 次都还在中继上。
const SELECTION_SETTLE: Duration = Duration::from_secs(1);

/// 定时检查看到的一拍。
#[derive(Debug)]
pub(super) struct Seen {
    pub(super) conn_id: usize,
    /// 选中的路径是中继。
    pub(super) relay: bool,
    /// 连接信息里的直连候选，见 [`direct_candidates`]。
    pub(super) candidates: BTreeSet<SocketAddr>,
    /// 这条连接到目前为止收发的字节数。
    pub(super) traffic: u64,
    /// 开着的插件端口流（TCP 和 UDP）。它们背后是终端、游戏、语音这类会话，断了调用方不会自己重来；
    /// HTTP 的流断了，客户端会换条连接重试。
    pub(super) sessions: usize,
    pub(super) at: Instant,
}

/// 什么时候该重拨：只看定时检查报上来的一拍拍，自己不碰连接，测试直接喂时间和状态。
#[derive(Debug, Default)]
pub(super) struct StickyRelay {
    /// 当前连接从哪一拍起一直走中继。
    relay_since: Option<(usize, Instant)>,
    /// 上一拍的收发字节数，算空闲用。
    last_traffic: Option<(usize, Instant, u64)>,
    /// 这段中继期里重拨过几次，最后一次在什么时候。
    redials: u32,
    last_redial: Option<Instant>,
    /// 重拨时的直连候选。之后的候选不全在里面，就是有了没试过的。
    tried: BTreeSet<SocketAddr>,
    /// 这个客户端重拨过的总次数，给日志和指标。
    total: u32,
}

impl StickyRelay {
    /// 记下这一拍，返回要不要重拨。返回真时已经记作重拨过一次。
    pub(super) fn observe(&mut self, seen: &Seen) -> bool {
        if !seen.relay {
            *self = Self {
                total: self.total,
                ..Self::default()
            };
            return false;
        }
        let since = match self.relay_since {
            Some((conn_id, since)) if conn_id == seen.conn_id => since,
            _ => {
                self.relay_since = Some((seen.conn_id, seen.at));
                seen.at
            }
        };
        let quiet = self.quiet(seen);
        // 初始那次建连已经把首包发给了全部候选，所以“没试过”只能近似：有直连候选就算，重拨过之后来了新的才重新给名额。
        if seen.candidates.is_empty() {
            return false;
        }
        if !seen.candidates.is_subset(&self.tried) {
            self.redials = 0;
            self.last_redial = None;
        }
        let due = seen.at.saturating_duration_since(since) >= RELAY_ONLY_BEFORE_REDIAL
            && self.redials < MAX_REDIALS
            && self
                .last_redial
                .is_none_or(|last| seen.at.saturating_duration_since(last) >= REDIAL_BACKOFF);
        if !due || !quiet || seen.sessions > 0 {
            return false;
        }
        self.redials += 1;
        self.total = self.total.saturating_add(1);
        self.last_redial = Some(seen.at);
        self.tried.clone_from(&seen.candidates);
        true
    }

    /// 从上一拍到这一拍，这条连接平均每秒收发不超过 [`QUIET_BYTES_PER_SEC`]。换了连接的第一拍不算空闲。
    fn quiet(&mut self, seen: &Seen) -> bool {
        let previous = self
            .last_traffic
            .replace((seen.conn_id, seen.at, seen.traffic));
        let Some((conn_id, at, traffic)) = previous else {
            return false;
        };
        let millis = seen.at.saturating_duration_since(at).as_millis();
        conn_id == seen.conn_id
            && millis > 0
            && u128::from(seen.traffic.saturating_sub(traffic)) * 1000
                <= u128::from(QUIET_BYTES_PER_SEC) * millis
    }

    pub(super) fn total_redials(&self) -> u32 {
        self.total
    }
}

/// 连接信息里的直连候选：IP 地址，去掉代理 TUN 的假地址（经代理绕一圈的“直连”，不值得为它打断连接）。
///
/// 只看建连用的连接信息：重拨时 iroh 把首包发给的就是这些（加上它记得的 QNT 地址）。QNT 报上来的地址 iroh
/// 自己每分钟打一次洞，不靠重拨；要把它们也算进来，从 `Endpoint::remote_info` 取，加在这里。
fn direct_candidates(server: &EndpointAddr) -> BTreeSet<SocketAddr> {
    server
        .ip_addrs()
        .filter(|addr| !is_fake_ip(addr.ip()))
        .copied()
        .collect()
}

impl TunnelClient {
    /// 连接一直走中继、连接信息里又有直连候选时，趁空闲断开重拨，给直连再一次机会。返回是否开始了重拨。
    ///
    /// 定时检查确认连接通了之后调用：时长、空闲和次数的判断见 [`StickyRelay`]。重拨期间的新请求等新连接；
    /// 重拨失败和别的建连失败一样，状态报断开、按退避重连。
    pub fn redial_if_stuck_on_relay(&self) -> bool {
        let Some(conn) = self.inner.live_connection() else {
            return false;
        };
        let paths = conn.paths();
        let Some(relay) = paths
            .iter()
            .find(|path| path.is_selected())
            .map(|path| path.is_relay())
        else {
            return false;
        };
        let stats = conn.stats();
        let streams = self.inner.streams.snapshot();
        let seen = Seen {
            conn_id: conn.stable_id(),
            relay,
            candidates: direct_candidates(&lock(&self.inner.server)),
            traffic: stats.udp_tx.bytes.saturating_add(stats.udp_rx.bytes),
            sessions: streams.port + streams.udp,
            at: suspend::now(self.inner.clock.as_deref()).0,
        };
        if !lock(&self.inner.sticky).observe(&seen) {
            return false;
        }
        let mut current = lock(&self.inner.attempt);
        let still_current = current.as_ref().is_some_and(|attempt| {
            matches!(attempt.outcome(), Some(Ok(live)) if live.stable_id() == seen.conn_id)
        });
        if !still_current {
            return false;
        }
        info!(
            candidates = seen.candidates.len(),
            "隧道一直走中继，断开重拨一次，让直连再试"
        );
        let client = self.clone();
        *current = Some(Attempt::spawn(
            true,
            true,
            &self.inner,
            client.redial_off_relay(conn),
        ));
        true
    }

    async fn redial_off_relay(self, old: Connection) -> Result<Connection, TunnelError> {
        self.inner.set_status(TunnelStatus::Connecting);
        old.close(CLOSE_NORMAL, b"redial off relay");
        old.closed().await;
        tokio::time::sleep(SELECTION_SETTLE).await;
        self.dial_replacement(None).await
    }
}

#[cfg(test)]
#[path = "sticky_tests.rs"]
mod tests;
