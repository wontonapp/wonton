//! ADR-0032：只采当前选中路径的增量；ACK-only 包不计入字节分母，避免下载时稀释丢包率。

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use iroh::TransportAddr;
use wonton_protocol::ConnectionQuality;
use wonton_tunnel::{PathInfo, PathKind};

const INTERVAL: Duration = Duration::from_secs(1);
const WINDOW: Duration = Duration::from_secs(30);
const MIN_PACKETS: u64 = 50;
const POOR_LOSS: u16 = 50;
const POOR_RTT: Duration = Duration::from_millis(800);
const POOR_HOLD: Duration = Duration::from_secs(10);
const GOOD_LOSS: u16 = 20;
const GOOD_RTT: Duration = Duration::from_millis(500);
const GOOD_HOLD: Duration = Duration::from_secs(30);
const CAPACITY: usize = 31;

#[derive(Clone, Copy, Debug, Default)]
struct Counters {
    packets: u64,
    bytes: u64,
    lost_bytes: u64,
}

impl Counters {
    fn from_path(path: &PathInfo) -> Self {
        Self {
            packets: path.sent_packets,
            bytes: path.sent_bytes,
            lost_bytes: path.lost_bytes,
        }
    }

    fn restarted(self, previous: Self) -> bool {
        self.packets < previous.packets
            || self.bytes < previous.bytes
            || self.lost_bytes < previous.lost_bytes
    }

    fn loss(self) -> u16 {
        if self.bytes == 0 {
            return if self.lost_bytes == 0 { 0 } else { 1_000 };
        }
        ((u128::from(self.lost_bytes) * 1_000 / u128::from(self.bytes)).min(1_000)) as u16
    }
}

#[derive(Debug)]
struct Sample {
    at: Instant,
    counters: Counters,
    rtt: Duration,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Reading {
    pub(super) quality: ConnectionQuality,
    pub(super) loss_per_mille: Option<u16>,
    pub(super) rtt_ms: Option<u32>,
}

#[derive(Debug)]
pub(super) struct Quality {
    path: Option<(usize, PathKind, TransportAddr)>,
    seen: Counters,
    sampled_at: Option<Instant>,
    samples: VecDeque<Sample>,
    reading: Reading,
    candidate: Option<(ConnectionQuality, Instant)>,
    #[cfg(feature = "test-hooks")]
    clock: Option<(Instant, Instant)>,
}

impl Default for Quality {
    fn default() -> Self {
        Self {
            path: None,
            seen: Counters::default(),
            sampled_at: None,
            samples: VecDeque::with_capacity(CAPACITY),
            reading: Reading::default(),
            candidate: None,
            #[cfg(feature = "test-hooks")]
            clock: None,
        }
    }
}

impl Quality {
    pub(super) fn observe(&mut self, now: Instant, path: &PathInfo) -> Reading {
        #[cfg(feature = "test-hooks")]
        let now = self.clock.map_or(now, |(_, current)| current);
        let counters = Counters::from_path(path);
        let changed = self.path.as_ref().is_none_or(|(connection, kind, remote)| {
            *connection != path.connection_id || *kind != path.kind || remote != &path.remote
        });
        let gap = self.sampled_at.map(|last| now.duration_since(last));
        if changed || counters.restarted(self.seen) || gap.is_some_and(|gap| gap > WINDOW) {
            self.reset();
            self.path = Some((path.connection_id, path.kind, path.remote.clone()));
            self.seen = counters;
            self.sampled_at = Some(now);
            return self.reading;
        }
        if gap.is_some_and(|gap| gap < INTERVAL) {
            return self.reading;
        }
        let delta = Counters {
            packets: counters.packets - self.seen.packets,
            bytes: counters.bytes - self.seen.bytes,
            lost_bytes: counters.lost_bytes - self.seen.lost_bytes,
        };
        self.seen = counters;
        self.sampled_at = Some(now);
        self.samples.push_back(Sample {
            at: now,
            counters: delta,
            rtt: path.rtt,
        });
        while self
            .samples
            .front()
            .is_some_and(|sample| now.duration_since(sample.at) >= WINDOW)
        {
            self.samples.pop_front();
        }
        self.evaluate(now, delta, path.rtt)
    }

    fn evaluate(&mut self, now: Instant, recent: Counters, recent_rtt: Duration) -> Reading {
        let mut total = Counters::default();
        let mut rtts = [Duration::ZERO; CAPACITY];
        for (index, sample) in self.samples.iter().enumerate() {
            total.packets += sample.counters.packets;
            total.bytes += sample.counters.bytes;
            total.lost_bytes += sample.counters.lost_bytes;
            rtts[index] = sample.rtt;
        }
        if total.packets < MIN_PACKETS || total.bytes == 0 {
            self.candidate = None;
            self.reading = Reading::default();
            return self.reading;
        }
        let rtts = &mut rtts[..self.samples.len()];
        let middle = rtts.len() / 2;
        let (_, rtt, _) = rtts.select_nth_unstable(middle);
        let loss = total.loss();
        self.reading.loss_per_mille = Some((loss + 2) / 5 * 5);
        self.reading.rtt_ms =
            Some(u32::try_from((rtt.as_millis() + 5) / 10 * 10).unwrap_or(u32::MAX));
        let desired = if loss >= POOR_LOSS || *rtt >= POOR_RTT {
            Some(ConnectionQuality::Poor)
        } else if loss < GOOD_LOSS && *rtt < GOOD_RTT {
            Some(ConnectionQuality::Good)
        } else {
            None
        };
        let poor_now = recent.loss() >= POOR_LOSS || recent_rtt >= POOR_RTT;
        self.follow(desired, now, poor_now);
        self.reading
    }

    pub(super) fn reset(&mut self) -> Reading {
        self.path = None;
        self.seen = Counters::default();
        self.sampled_at = None;
        self.samples.clear();
        self.candidate = None;
        self.reading = Reading::default();
        self.reading
    }

    fn follow(&mut self, desired: Option<ConnectionQuality>, now: Instant, poor_now: bool) {
        let Some(desired) = desired.filter(|desired| *desired != self.reading.quality) else {
            self.candidate = None;
            return;
        };
        let hold = if desired == ConnectionQuality::Poor {
            POOR_HOLD
        } else {
            GOOD_HOLD
        };
        match self.candidate {
            Some((candidate, since)) if candidate == desired => {
                // 窗口持续差、最新区间也仍差才切 Poor；一秒干净样本不抹掉低流量链路的连续窗口证据。
                if now.duration_since(since) >= hold
                    && (desired != ConnectionQuality::Poor || poor_now)
                {
                    self.reading.quality = desired;
                    self.candidate = None;
                    tracing::info!(quality = ?desired, loss_per_mille = self.reading.loss_per_mille,
                        rtt_ms = self.reading.rtt_ms, "connection quality changed");
                }
            }
            _ => self.candidate = Some((desired, now)),
        }
    }

    #[cfg(feature = "test-hooks")]
    fn set_time(&mut self, elapsed: Duration) {
        let (origin, previous) = match self.clock {
            Some(clock) => clock,
            None => {
                self.reset();
                let now = Instant::now();
                (now, now)
            }
        };
        let now = origin + elapsed;
        assert!(now >= previous, "quality clock moved backwards");
        self.clock = Some((origin, now));
    }
}

#[cfg(feature = "test-hooks")]
impl super::HomeLink {
    pub(crate) async fn test_sample_quality(
        &self,
        elapsed: Duration,
    ) -> wonton_protocol::HomeViewModel {
        self.lock_quality().set_time(elapsed);
        let Some(proxy) = self.view().proxy else {
            return self.view();
        };
        if let Some((endpoint, client)) = self.current_link().await {
            *self.lock_stats_at() = None;
            self.publish(&client.status(), &endpoint, &client, &proxy);
        }
        self.view()
    }
}

#[cfg(test)]
mod tests;
