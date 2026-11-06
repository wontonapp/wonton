//! 插件通知的频率限制（ADR-0047）：每个插件每分钟最多 10 条、滚动 24 小时最多 200 条，超了的丢弃。计数只在内存里，
//! 核心重启后重新计；插件是主人自己装的，不值得为此写状态。
//!
//! 从"没超"变成"超了"的那一刻要提示主人一次，同一段超限期间不再提示：这段状态也在这里，有请求被放行时重新计。

use std::collections::{BTreeMap, VecDeque};
use std::time::{Duration, Instant};

use crate::plugin::PluginId;

/// 每分钟最多多少条。
pub const PER_MINUTE: usize = 10;
/// 滚动 24 小时最多多少条。
pub const PER_DAY: usize = 200;

const MINUTE: Duration = Duration::from_secs(60);
const DAY: Duration = Duration::from_secs(24 * 60 * 60);

/// 被拒的一条通知。
pub struct Denied {
    /// 最早什么时候能再发：压住窗口的那条旧记录滑出去的时间。
    pub retry_after: Duration,
    /// 这段超限期间还没有提示过主人：调用方发一条提示，之后 `first` 都是假。
    pub first: bool,
}

#[derive(Default)]
struct Windows {
    minute: VecDeque<Instant>,
    day: VecDeque<Instant>,
    /// 提示过了：这段超限期间不再提示。
    noticed: bool,
}

#[derive(Default)]
pub struct Limiter {
    plugins: std::sync::Mutex<BTreeMap<PluginId, Windows>>,
}

impl Limiter {
    /// 要发一条通知：放得下返回真并记下，超了返回 [`Denied`]，那条被丢弃。
    pub fn admit(&self, id: &PluginId) -> Result<(), Denied> {
        let now = Instant::now();
        let mut plugins = self.plugins.lock().expect("通知频率表锁中毒");
        admit_at(plugins.entry(id.clone()).or_default(), now)
    }
}

/// 纯逻辑，测试直接喂时间。
fn admit_at(w: &mut Windows, now: Instant) -> Result<(), Denied> {
    drain(&mut w.minute, now, MINUTE);
    drain(&mut w.day, now, DAY);
    // 24 小时的先看：它也超了的时候，每分钟腾出位置也发不出去。
    if w.day.len() >= PER_DAY {
        return Err(denied(w, w.day.front().copied(), DAY, now));
    }
    if w.minute.len() >= PER_MINUTE {
        return Err(denied(w, w.minute.front().copied(), MINUTE, now));
    }
    w.minute.push_back(now);
    w.day.push_back(now);
    w.noticed = false;
    Ok(())
}

/// 滑出窗口的记录不再占名额。
fn drain(window: &mut VecDeque<Instant>, now: Instant, span: Duration) {
    while window
        .front()
        .is_some_and(|at| now.duration_since(*at) >= span)
    {
        window.pop_front();
    }
}

/// 拒掉，并把"这段超限期间要提示一次"记下来。
fn denied(w: &mut Windows, front: Option<Instant>, span: Duration, now: Instant) -> Denied {
    let retry_after = front
        .and_then(|at| span.checked_sub(now.duration_since(at)))
        .unwrap_or(span);
    let first = !w.noticed;
    w.noticed = true;
    Denied { retry_after, first }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_eleventh_within_a_minute_is_denied_until_the_first_slides_out() {
        let mut w = Windows::default();
        let start = Instant::now();
        for i in 0..PER_MINUTE as u64 {
            assert!(
                admit_at(&mut w, start + Duration::from_secs(i)).is_ok(),
                "第 {} 条",
                i + 1
            );
        }
        let err = admit_at(&mut w, start + Duration::from_secs(11)).unwrap_err();
        assert_eq!(err.retry_after, Duration::from_secs(49));
        assert!(err.first, "第一次超限要提示");
        // 同一段超限期间不再提示。
        let again = admit_at(&mut w, start + Duration::from_secs(12)).unwrap_err();
        assert!(!again.first);
        // 最早的一条滑出去就能再发，提示状态重新计。
        assert!(admit_at(&mut w, start + MINUTE).is_ok());
        for i in 1..PER_MINUTE as u64 {
            assert!(admit_at(&mut w, start + MINUTE + Duration::from_secs(i)).is_ok());
        }
        let after = admit_at(&mut w, start + MINUTE + Duration::from_secs(10)).unwrap_err();
        assert!(after.first, "放行过一次后再次超限，重新提示");
    }

    #[test]
    fn the_daily_limit_reports_a_longer_wait() {
        let mut w = Windows::default();
        let start = Instant::now();
        // 每 7 秒一条：任何一分钟里都不到 10 条，分钟的限制不触发。
        for i in 0..PER_DAY as u64 {
            assert!(admit_at(&mut w, start + Duration::from_secs(i * 7)).is_ok());
        }
        // 200 条要 19 分钟以上，所以最后一条之后一分钟内就是天的恢复点在起作用。
        let err = admit_at(&mut w, start + Duration::from_secs(PER_DAY as u64 * 7)).unwrap_err();
        assert_eq!(
            err.retry_after,
            DAY - Duration::from_secs(PER_DAY as u64 * 7)
        );
    }

    #[test]
    fn a_full_day_window_stays_denied_when_the_minute_one_frees() {
        let mut w = Windows::default();
        let start = Instant::now();
        for i in 0..PER_DAY as u64 {
            assert!(admit_at(&mut w, start + Duration::from_secs(i * 7)).is_ok());
        }
        // 最后一条过去一分钟以后，每分钟的限制已经空了，24 小时的还压着：等到的是天的恢复点。
        let now = start + Duration::from_secs(PER_DAY as u64 * 7) + MINUTE + Duration::from_secs(1);
        let err = admit_at(&mut w, now).unwrap_err();
        assert_eq!(
            err.retry_after,
            DAY - Duration::from_secs(PER_DAY as u64 * 7 + 61)
        );
    }
}
