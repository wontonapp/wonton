//! 按设备限速（ADR-0065）：一分钟内的次数，滑动窗口。计数只在内存里，核心重启从头算。

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

use super::Clock;

const WINDOW_MS: u64 = 60 * 1000;

pub(super) struct RateLimit {
    per_minute: usize,
    now: Clock,
    seen: Mutex<HashMap<String, VecDeque<u64>>>,
}

impl RateLimit {
    pub fn new(per_minute: usize, now: Clock) -> Self {
        Self {
            per_minute,
            now,
            seen: Mutex::new(HashMap::new()),
        }
    }

    /// 这台设备还能不能再取一次；能的话记下这一次。
    pub fn take(&self, device: &str) -> bool {
        let now = (self.now)();
        let mut seen = self.seen.lock().expect("限速锁中毒");
        // 顺手清掉窗口外的记录，设备多了表也不会一直长。
        seen.retain(|_, times| {
            while times
                .front()
                .is_some_and(|at| now.saturating_sub(*at) >= WINDOW_MS)
            {
                times.pop_front();
            }
            !times.is_empty()
        });
        let times = seen.entry(device.to_owned()).or_default();
        if times.len() >= self.per_minute {
            return false;
        }
        times.push_back(now);
        true
    }
}
