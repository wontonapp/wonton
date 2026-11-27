use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use super::{REDIAL_BACKOFF, RELAY_ONLY_BEFORE_REDIAL, Seen, StickyRelay};

const TICK: Duration = Duration::from_secs(5);

fn candidates(addrs: &[&str]) -> BTreeSet<SocketAddr> {
    addrs.iter().map(|addr| addr.parse().unwrap()).collect()
}

/// 定时检查的一拍：连接 `conn_id`，空闲（收发只有心跳那点字节），没开插件端口。
struct Line {
    tracker: StickyRelay,
    start: Instant,
    at: Instant,
    traffic: u64,
}

impl Line {
    fn new() -> Self {
        let start = Instant::now();
        Self {
            tracker: StickyRelay::default(),
            start,
            at: start,
            traffic: 0,
        }
    }

    /// 往后走一拍，期间收发 `bytes` 字节，返回这一拍要不要重拨。
    fn tick(
        &mut self,
        conn_id: usize,
        relay: bool,
        addrs: &BTreeSet<SocketAddr>,
        bytes: u64,
    ) -> bool {
        self.tick_with(conn_id, relay, addrs, bytes, 0)
    }

    fn tick_with(
        &mut self,
        conn_id: usize,
        relay: bool,
        addrs: &BTreeSet<SocketAddr>,
        bytes: u64,
        sessions: usize,
    ) -> bool {
        self.at += TICK;
        self.traffic += bytes;
        self.tracker.observe(&Seen {
            conn_id,
            relay,
            candidates: addrs.clone(),
            traffic: self.traffic,
            sessions,
            at: self.at,
        })
    }

    /// 空闲地一拍拍走到 `until`（从第一拍算起），返回其间要求重拨的拍数。
    fn idle_until(
        &mut self,
        conn_id: usize,
        relay: bool,
        addrs: &BTreeSet<SocketAddr>,
        until: Duration,
    ) -> usize {
        let mut redials = 0;
        while self.at.duration_since(self.start) < until {
            redials += usize::from(self.tick(conn_id, relay, addrs, 300));
        }
        redials
    }
}

/// ① 一直走中继超过时限、又有直连候选：空闲的那一拍重拨一次，之后不再连着重拨。
#[test]
fn a_line_stuck_on_relay_with_a_direct_candidate_redials_once() {
    let addrs = candidates(&["192.0.2.10:4433"]);
    let mut line = Line::new();
    assert_eq!(
        line.idle_until(1, true, &addrs, RELAY_ONLY_BEFORE_REDIAL - TICK),
        0,
        "不到时限不重拨"
    );
    assert_eq!(
        line.idle_until(1, true, &addrs, RELAY_ONLY_BEFORE_REDIAL + TICK * 2),
        1
    );
    assert_eq!(line.tracker.total_redials(), 1);
}

/// ② 一直走中继，但没有直连候选（只有中继地址）：重拨也只会再连上中继，不动。
#[test]
fn a_relay_only_line_without_direct_candidates_is_left_alone() {
    let mut line = Line::new();
    assert_eq!(
        line.idle_until(1, true, &BTreeSet::new(), RELAY_ONLY_BEFORE_REDIAL * 6),
        0
    );
    assert_eq!(line.tracker.total_redials(), 0);
}

/// ③ 重拨过一次、新连接还在中继上：退避期内不再重拨；过了退避给第二次，之后这段中继期不再给。
#[test]
fn after_a_redial_the_next_one_waits_for_the_backoff_and_then_stops() {
    let addrs = candidates(&["192.0.2.10:4433"]);
    let mut line = Line::new();
    while !line.tick(1, true, &addrs, 300) {}
    let first = line.at;
    // 重拨后的新连接还是中继。
    while line.at.duration_since(first) < REDIAL_BACKOFF - TICK {
        assert!(!line.tick(2, true, &addrs, 300), "退避期内不再重拨");
    }
    assert!(line.tick(2, true, &addrs, 300), "过了退避给第二次");
    let second = line.at;
    while line.at.duration_since(second) < REDIAL_BACKOFF * 6 {
        assert!(
            !line.tick(3, true, &addrs, 300),
            "两次都没换到直连，这段中继期不再重拨"
        );
    }
    assert_eq!(line.tracker.total_redials(), 2);
}

/// ④ 走直连的连接不重拨，不管多久、有没有候选。
#[test]
fn a_direct_line_never_redials() {
    let addrs = candidates(&["192.0.2.10:4433"]);
    let mut line = Line::new();
    assert_eq!(
        line.idle_until(1, false, &addrs, RELAY_ONLY_BEFORE_REDIAL * 6),
        0
    );
}

/// 中继期按连接算，中间换到过直连就从头算：短暂落回中继不攒时长。
#[test]
fn going_direct_restarts_the_relay_clock() {
    let addrs = candidates(&["192.0.2.10:4433"]);
    let mut line = Line::new();
    line.idle_until(1, true, &addrs, RELAY_ONLY_BEFORE_REDIAL - TICK * 2);
    assert!(!line.tick(1, false, &addrs, 300));
    let resumed = line.at;
    while line.at.duration_since(resumed) < RELAY_ONLY_BEFORE_REDIAL - TICK {
        assert!(!line.tick(1, true, &addrs, 300), "换过直连，中继时长从头算");
    }
}

/// 有数据在传、或开着插件端口时先不重拨：等空闲的那一拍。
#[test]
fn a_busy_line_waits_for_a_quiet_moment() {
    let addrs = candidates(&["192.0.2.10:4433"]);
    let mut line = Line::new();
    line.idle_until(1, true, &addrs, RELAY_ONLY_BEFORE_REDIAL - TICK);
    for _ in 0..20 {
        assert!(!line.tick(1, true, &addrs, 5 * 1024 * 1024), "下载中不打断");
    }
    for _ in 0..20 {
        assert!(
            !line.tick_with(1, true, &addrs, 300, 1),
            "开着插件端口不打断"
        );
    }
    assert!(line.tick(1, true, &addrs, 300), "空下来就重拨");
}

/// 用完两次之后，连接信息里来了没试过的直连候选：再给机会。
#[test]
fn new_direct_candidates_earn_a_fresh_try() {
    let first = candidates(&["192.0.2.10:4433"]);
    let mut line = Line::new();
    let mut redials = 0;
    for conn in 1..=4 {
        let began = line.at;
        while line.at.duration_since(began) < REDIAL_BACKOFF + RELAY_ONLY_BEFORE_REDIAL {
            redials += usize::from(line.tick(conn, true, &first, 300));
        }
    }
    assert_eq!(redials, 2);
    let more = candidates(&["192.0.2.10:4433", "192.0.2.11:4433"]);
    let began = line.at;
    let mut fresh = 0;
    while line.at.duration_since(began) < RELAY_ONLY_BEFORE_REDIAL + TICK * 2 {
        fresh += usize::from(line.tick(5, true, &more, 300));
    }
    assert_eq!(fresh, 1);
}
