//! 重连的节奏：连续失败时等待加倍，上下浮动 20%（ADR-0032）。

use std::time::Duration;

use tokio::time::Instant;

const FIRST: Duration = Duration::from_secs(1);
const MAX: Duration = Duration::from_secs(30);

#[derive(Debug, Default)]
pub(crate) struct Pacing {
    failures: u32,
    not_before: Option<Instant>,
}

impl Pacing {
    /// 下一次建连最早什么时候开始。
    pub(crate) fn not_before(&self) -> Option<Instant> {
        self.not_before
    }

    pub(crate) fn failed(&mut self) {
        let doubled = FIRST.saturating_mul(1u32.checked_shl(self.failures).unwrap_or(u32::MAX));
        self.failures = self.failures.saturating_add(1);
        self.not_before = Some(Instant::now() + jittered(doubled.min(MAX)));
    }

    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }
}

/// 上下浮动 20%。一台服务器只有几台手机，没有惊群，不需要全抖动；全抖动还会把等待压到接近 0。
fn jittered(wait: Duration) -> Duration {
    let mut byte = [0u8; 1];
    // 取不到随机数时不浮动，只是少了一点错峰。
    let random = getrandom::fill(&mut byte).map_or(128, |()| u32::from(byte[0]));
    wait * (80 + random * 40 / 255) / 100
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wait_after(failures: u32) -> Duration {
        let mut pacing = Pacing::default();
        for _ in 0..failures {
            pacing.failed();
        }
        pacing.not_before().unwrap() - Instant::now()
    }

    #[tokio::test(start_paused = true)]
    async fn waits_double_up_to_the_limit_and_stay_within_the_jitter() {
        let seconds = |s: f64| Duration::from_secs_f64(s);
        for (failures, nominal) in [(1, 1.0), (2, 2.0), (3, 4.0), (6, 30.0), (40, 30.0)] {
            let wait = wait_after(failures);
            assert!(
                wait >= seconds(nominal * 0.8) && wait <= seconds(nominal * 1.2),
                "第 {failures} 次失败后等了 {wait:?}"
            );
        }
    }
}
