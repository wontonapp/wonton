//! 等健康门的时限：装不动 300 秒的插件（大模型类要装载几分钟）按它声明的 `start_period` 放宽，
//! 其余照旧用配置的安装时限。安装、启动、升级、局域网重建和加 Bot 用同一条规则。

use std::time::Duration;

use bollard::models::HealthConfig;
use tokio::time::Instant;

use super::render::Deployment;

/// `start_period` 只是"启动后多久内检查失败不算数"，之后还要按 `interval × retries` 才判不健康，
/// 再留出容器调度和程序初始化的时间，余量取 5 分钟。
const START_PERIOD_MARGIN: Duration = Duration::from_secs(5 * 60);
/// 等待的上限：声明得再长也不能让主人无限等，超时照实报错（失败即停）。60 分钟盖住大模型类插件
/// 实测的装载时间。
const MAX_WAIT: Duration = Duration::from_secs(60 * 60);

/// 插件的等待时限：渲染计划（已经过 `rewrite_image_healthchecks`，镜像自带的 healthcheck 也算）里各
/// 容器 `healthcheck.start_period` 的最大值加余量，下限 `install_timeout`，上限 max(60 分钟,
/// `install_timeout`)；没有容器写 `start_period` 时就是 `install_timeout`。
pub(super) fn deadline(plan: &Deployment, install_timeout: Duration, now: Instant) -> Instant {
    deadline_from_configs(
        plan.containers.iter().map(|c| c.body.healthcheck.as_ref()),
        install_timeout,
        now,
    )
}

/// 启动现存的容器时没有渲染计划：生效的 healthcheck（安装时把镜像自带的并了进去）在容器配置里。
pub(super) fn deadline_from_configs<'a>(
    configs: impl IntoIterator<Item = Option<&'a HealthConfig>>,
    install_timeout: Duration,
    now: Instant,
) -> Instant {
    let longest = configs
        .into_iter()
        .filter_map(|check| start_period(check?))
        .max();
    let Some(period) = longest.filter(|period| !period.is_zero()) else {
        return now + install_timeout;
    };
    // 声明得比上限还长时按上限等；`install_timeout` 配得比 60 分钟长时以它为准。
    now + (period + START_PERIOD_MARGIN).clamp(install_timeout, MAX_WAIT.max(install_timeout))
}

/// Docker 的 `start_period` 为 0 时没有宽限期，和没写一样。
fn start_period(check: &HealthConfig) -> Option<Duration> {
    let nanos = check.start_period?;
    (nanos > 0).then(|| Duration::from_nanos(nanos as u64))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limit(start_period: Option<i64>, install_timeout: Duration) -> Duration {
        let now = Instant::now();
        let check = start_period.map(|nanos| HealthConfig {
            start_period: Some(nanos),
            ..Default::default()
        });
        deadline_from_configs([check.as_ref()], install_timeout, now) - now
    }

    #[test]
    fn without_a_start_period_the_limit_is_the_install_timeout() {
        assert_eq!(
            limit(None, Duration::from_secs(300)),
            Duration::from_secs(300)
        );
    }

    #[test]
    fn the_longest_start_period_plus_margin_wins_within_bounds() {
        assert_eq!(
            limit(Some(45 * 60 * 1_000_000_000), Duration::from_secs(300)),
            Duration::from_secs(50 * 60)
        );
        // 余量碰到下限：start_period 短时至少等 install_timeout。
        assert_eq!(
            limit(Some(10_000_000_000), Duration::from_secs(3600)),
            Duration::from_secs(3600)
        );
    }

    #[test]
    fn a_start_period_beyond_the_cap_is_clamped() {
        assert_eq!(
            limit(Some(90 * 60 * 1_000_000_000), Duration::from_secs(300)),
            Duration::from_secs(60 * 60)
        );
        // install_timeout 配得比上限长时以它为准。
        assert_eq!(
            limit(Some(90 * 60 * 1_000_000_000), Duration::from_secs(61 * 60)),
            Duration::from_secs(61 * 60)
        );
    }

    #[test]
    fn a_zero_start_period_is_unset() {
        assert_eq!(
            limit(Some(0), Duration::from_secs(300)),
            Duration::from_secs(300)
        );
    }
}
