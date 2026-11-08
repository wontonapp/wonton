//! 一个流向的收端：完整包直接交出，两片按包编号拼回。
//!
//! 等待中的半包有上限也会过期；拼完、过期或被挤掉的包编号记一段时间，迟到片和重复片直接丢掉，不会和
//! 后来的包错拼（`wire::datagram` 把这些生命周期留给调用方）。

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};

use bytes::Bytes;

use crate::wire::datagram::{Fragment, FragmentIndex, Frame, join};

/// 两片相隔超过这么久就不再等：数据报不重传，晚到的半包对实时流没用。ADR-0072 测到换路径时丢包集中在
/// 1–3 秒里，那时的半包本来就等不到另一半。
pub(crate) const FRAGMENT_TTL: Duration = Duration::from_secs(1);
/// 每个流向同时等的半包上限，满了挤掉最旧的。按 ADR-0072 的 100 Mbit/s、1400 字节算，每毫秒约 9 个包，
/// 半包只在丢了一片时才留下，64 个足够盖住乱序。
pub(crate) const MAX_PENDING: usize = 64;
/// 记住结束了的包编号这么久，比 [`FRAGMENT_TTL`] 长，丢掉它们的迟到片。
const SETTLED_TTL: Duration = Duration::from_secs(4);
const MAX_SETTLED: usize = 1024;

#[derive(Debug)]
struct Half {
    index: FragmentIndex,
    payload: Bytes,
    at: Instant,
}

#[derive(Debug, Default)]
pub(crate) struct Reassembler {
    pending: HashMap<u64, Half>,
    /// 半包进来的先后，过期和挤掉都从头取；表里可能留着已经拼完的旧项，取到时跳过。
    arrivals: VecDeque<(u64, Instant)>,
    settled: HashSet<u64>,
    settled_order: VecDeque<(u64, Instant)>,
}

impl Reassembler {
    /// `datagram` 是 `frame` 借用的那份数据报，完整包不复制，切出它的载荷。
    pub(crate) fn accept(
        &mut self,
        datagram: &Bytes,
        frame: Frame<'_>,
        now: Instant,
    ) -> Option<Bytes> {
        self.expire(now);
        let Some(fragment) = frame.fragment else {
            return Some(datagram.slice_ref(frame.payload));
        };
        let id = fragment.packet_id;
        if self.settled.contains(&id) {
            return None;
        }
        let Some(half) = self.pending.remove(&id) else {
            self.hold(id, fragment.index, datagram.slice_ref(frame.payload), now);
            return None;
        };
        if half.index == fragment.index {
            // 重复片不覆盖先到的那片。
            self.pending.insert(id, half);
            return None;
        }
        let held = Frame {
            flow: frame.flow,
            fragment: Some(Fragment {
                packet_id: id,
                index: half.index,
            }),
            payload: &half.payload,
        };
        let mut output = vec![0; held.payload.len() + frame.payload.len()];
        let joined = join(Some(held), Some(frame), &mut output)
            .ok()
            .flatten()
            .map(<[u8]>::len);
        self.settle(id, now);
        joined.map(|len| {
            output.truncate(len);
            Bytes::from(output)
        })
    }

    fn hold(&mut self, id: u64, index: FragmentIndex, payload: Bytes, now: Instant) {
        while self.pending.len() >= MAX_PENDING {
            let Some((oldest, _)) = self.arrivals.pop_front() else {
                break;
            };
            if self.pending.remove(&oldest).is_some() {
                self.settle(oldest, now);
            }
        }
        self.pending.insert(
            id,
            Half {
                index,
                payload,
                at: now,
            },
        );
        self.arrivals.push_back((id, now));
    }

    fn expire(&mut self, now: Instant) {
        while let Some(&(id, at)) = self.arrivals.front() {
            if now.duration_since(at) < FRAGMENT_TTL {
                break;
            }
            self.arrivals.pop_front();
            if self.pending.get(&id).is_some_and(|half| half.at == at) {
                self.pending.remove(&id);
                self.settle(id, now);
            }
        }
        while let Some(&(id, at)) = self.settled_order.front() {
            if now.duration_since(at) < SETTLED_TTL && self.settled_order.len() <= MAX_SETTLED {
                break;
            }
            self.settled_order.pop_front();
            self.settled.remove(&id);
        }
    }

    fn settle(&mut self, id: u64, now: Instant) {
        if self.settled.insert(id) {
            self.settled_order.push_back((id, now));
        }
    }

    #[cfg(test)]
    fn pending(&self) -> usize {
        self.pending.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::datagram::{FlowId, Packet, split};

    fn frames(flow: FlowId, id: u64, payload: &[u8], max: usize) -> Vec<Bytes> {
        let Packet::Pair(pair) = split(flow, id, payload, max).unwrap() else {
            panic!("expected two fragments");
        };
        pair.iter()
            .map(|frame| {
                let mut out = vec![0; frame.header_len() + frame.payload.len()];
                let len = frame.encode(&mut out, max).unwrap();
                out.truncate(len);
                Bytes::from(out)
            })
            .collect()
    }

    fn feed(r: &mut Reassembler, datagram: &Bytes, now: Instant) -> Option<Bytes> {
        r.accept(datagram, Frame::decode(datagram).unwrap(), now)
    }

    #[test]
    fn out_of_order_halves_join_once_and_late_copies_are_dropped() {
        let flow = FlowId::new(70).unwrap();
        let payload: Vec<u8> = (0..1500u32).map(|i| i as u8).collect();
        let parts = frames(flow, 9, &payload, 1000);
        let mut r = Reassembler::default();
        let now = Instant::now();
        assert_eq!(feed(&mut r, &parts[1], now), None);
        assert_eq!(feed(&mut r, &parts[1], now), None, "duplicate half");
        assert_eq!(feed(&mut r, &parts[0], now).as_deref(), Some(&payload[..]));
        assert_eq!(
            feed(&mut r, &parts[0], now),
            None,
            "late copy of a joined packet"
        );
        assert_eq!(r.pending(), 0);
    }

    #[test]
    fn halves_expire_and_the_pending_set_is_bounded() {
        let flow = FlowId::new(1).unwrap();
        let payload = vec![7u8; 1500];
        let mut r = Reassembler::default();
        let start = Instant::now();
        let stale = frames(flow, 0, &payload, 1000);
        assert_eq!(feed(&mut r, &stale[0], start), None);
        let later = start + FRAGMENT_TTL;
        assert_eq!(
            feed(&mut r, &stale[1], later),
            None,
            "the first half expired"
        );
        for id in 1..=(MAX_PENDING as u64 + 10) {
            feed(&mut r, &frames(flow, id, &payload, 1000)[0], later);
        }
        assert_eq!(r.pending(), MAX_PENDING);
        let evicted = frames(flow, 1, &payload, 1000);
        assert_eq!(
            feed(&mut r, &evicted[1], later),
            None,
            "evicted halves stay dropped"
        );
        let kept = frames(flow, MAX_PENDING as u64 + 10, &payload, 1000);
        assert_eq!(feed(&mut r, &kept[1], later).as_deref(), Some(&payload[..]));
    }
}
