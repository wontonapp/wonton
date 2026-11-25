//! 等健康门的时限：装不动 300 秒的插件（大模型类要装载几分钟）按它声明的 `start_period` 和健康检查
//! 自己的判定窗放宽，其余照旧用配置的安装时限。安装、启动、升级、局域网重建和加 Bot 用同一条规则；
//! 开机恢复和容器监视是串行核对的，用固定的安装时限（见 [`Gate`]）。

use std::time::Duration;

use bollard::models::HealthConfig;
use tokio::time::Instant;

use super::render::Deployment;

/// `start_period` 只是"启动后多久内检查失败不算数"，之后还要按 `interval × retries` 才判不健康，
/// 再留出容器调度和程序初始化的时间，余量取 5 分钟。
const START_PERIOD_MARGIN: Duration = Duration::from_secs(5 * 60);
/// Docker 的 `interval` 缺省 30 秒、`retries` 缺省 3 次。
const DEFAULT_INTERVAL: Duration = Duration::from_secs(30);
const DEFAULT_RETRIES: i64 = 3;
/// 等待的上限：声明得再长也不能让主人无限等，超时照实报错（失败即停）。60 分钟盖住大模型类插件
/// 实测的装载时间。
const MAX_WAIT: Duration = Duration::from_secs(60 * 60);

/// 重建容器时等健康门用哪条时限。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::plugin) enum Gate {
    /// 按插件声明的 `start_period` 放宽（见 [`deadline`]）：安装、启动、升级这类主人等着的长操作。
    StartPeriod,
    /// 固定 `install_timeout`，不看 `start_period`：开机恢复和容器监视是一个插件接一个插件串行核对的，
    /// 一个起不来的慢插件不能拖住排在后面的所有插件几十分钟。超时照实标成异常，容器留着，健康以后由
    /// 容器监视捡回来。
    InstallTimeout,
}

/// 等健康门的时限：从 [`Wait::started`] 算起等到 [`Wait::at`]，总共给了 [`Wait::budget`]。超时报告里
/// 写清等了多少，主人和作者才知道是门槛没给够还是服务真起不来。
#[derive(Clone, Copy, Debug)]
pub struct Wait {
    pub at: Instant,
    pub budget: Duration,
    pub started: Instant,
}

impl Gate {
    /// 现在起的时限。
    pub(super) fn deadline(
        self,
        plan: &Deployment,
        install_timeout: Duration,
        now: Instant,
    ) -> Wait {
        match self {
            Self::StartPeriod => deadline(plan, install_timeout, now),
            Self::InstallTimeout => Wait {
                at: now + install_timeout,
                budget: install_timeout,
                started: now,
            },
        }
    }
}

/// 插件的等待时限：渲染计划（已经过 `rewrite_image_healthchecks`，镜像自带的 healthcheck 也算）里各
/// 容器自己的判定窗（`start_period + interval × retries`）的最大值、或最长 `start_period` 加余量，
/// 取大的；下限 `install_timeout`，上限 max(60 分钟, `install_timeout`)。没写 `start_period` 的检查只
/// 按自己的判定窗参与，不加余量。
pub(super) fn deadline(plan: &Deployment, install_timeout: Duration, now: Instant) -> Wait {
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
) -> Wait {
    let mut longest = Duration::ZERO;
    let mut window = Duration::ZERO;
    for check in configs.into_iter().flatten() {
        let period = start_period(check).unwrap_or_default();
        longest = longest.max(period);
        window = window.max(period + retry_window(check));
    }
    // 没写 `start_period` 时 `longest` 为 0，5 分钟的余量不加；检查自己的判定窗照样计入，
    // 否则 `{interval: 60s, retries: 10}` 这种要在 Docker 判不健康前 10 分钟就放弃了。
    let retained = if longest.is_zero() {
        Duration::ZERO
    } else {
        longest + START_PERIOD_MARGIN
    };
    // 声明得比上限还长时按上限等；`install_timeout` 配得比 60 分钟长时以它为准。
    let budget = retained
        .max(window)
        .clamp(install_timeout, MAX_WAIT.max(install_timeout));
    Wait {
        at: now + budget,
        budget,
        started: now,
    }
}

/// Docker 的 `start_period` 为 0 时没有宽限期，和没写一样。
fn start_period(check: &HealthConfig) -> Option<Duration> {
    let nanos = check.start_period?;
    (nanos > 0).then(|| Duration::from_nanos(nanos as u64))
}

/// Docker 判不健康的退避：连续 `retries` 次探针失败、每次间隔 `interval` 才判（Dockerfile 的
/// HEALTHCHECK 文档）。单次探针的 `timeout` 不在内。
fn retry_window(check: &HealthConfig) -> Duration {
    let interval =
        Duration::from_nanos(check.interval.unwrap_or(DEFAULT_INTERVAL.as_nanos() as i64) as u64);
    let retries = check
        .retries
        .unwrap_or(DEFAULT_RETRIES)
        .max(1)
        .min(i64::from(u32::MAX)) as u32;
    interval * retries
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEC: Duration = Duration::from_secs(1);

    /// `start_period` 以秒给出，`interval` 和 `retries` 用调用方给的。
    fn limit(start_period: Option<i64>, interval: Option<i64>, retries: Option<i64>) -> Duration {
        let check =
            (start_period.is_some() || interval.is_some() || retries.is_some()).then(|| {
                HealthConfig {
                    start_period: start_period.map(|seconds| seconds * 1_000_000_000),
                    interval: interval.map(|seconds| seconds * 1_000_000_000),
                    retries,
                    ..Default::default()
                }
            });
        deadline_from_configs([check.as_ref()], Duration::from_secs(300), Instant::now()).budget
    }

    #[test]
    fn without_a_start_period_the_limit_is_the_install_timeout() {
        assert_eq!(limit(None, None, None), 300 * SEC);
    }

    #[test]
    fn the_longest_start_period_plus_margin_wins_within_bounds() {
        assert_eq!(limit(Some(45 * 60), None, None), 50 * 60 * SEC);
        // 余量碰到下限：start_period 短时至少等 install_timeout。
        let short = HealthConfig {
            start_period: Some(10 * 1_000_000_000),
            ..Default::default()
        };
        assert_eq!(
            deadline_from_configs([Some(&short)], 3600 * SEC, Instant::now()).budget,
            3600 * SEC
        );
    }

    #[test]
    fn a_start_period_beyond_the_cap_is_clamped() {
        assert_eq!(limit(Some(90 * 60), None, None), 60 * 60 * SEC);
        // install_timeout 配得比上限长时以它为准。
        let long = HealthConfig {
            start_period: Some(90 * 60 * 1_000_000_000),
            ..Default::default()
        };
        assert_eq!(
            deadline_from_configs([Some(&long)], 61 * 60 * SEC, Instant::now()).budget,
            61 * 60 * SEC
        );
    }

    #[test]
    fn a_zero_start_period_is_unset() {
        assert_eq!(limit(Some(0), Some(10), Some(10)), 300 * SEC);
    }

    #[test]
    fn the_health_checks_own_retry_window_is_covered() {
        // 2m + 5m 的余量比 2m + 30s × 20 = 12m 短，必须取 12m，否则还在 Docker 的判定窗里。
        assert_eq!(limit(Some(2 * 60), Some(30), Some(20)), 12 * 60 * SEC);
    }

    #[test]
    fn a_retry_window_without_a_start_period_still_counts() {
        // interval 60s × retries 10：Docker 10 分钟才判不健康，默认的 300 秒不能先放弃。
        assert_eq!(limit(None, Some(60), Some(10)), 10 * 60 * SEC);
    }

    #[test]
    fn a_short_retry_window_does_not_shorten_the_margin() {
        assert_eq!(limit(Some(50 * 60), Some(10), Some(2)), 55 * 60 * SEC);
    }

    #[test]
    fn the_retry_window_is_capped_like_the_start_period() {
        // 58m + 60s × 60 = 118m，超过 60 分钟上限。
        assert_eq!(limit(Some(58 * 60), Some(60), Some(60)), 60 * 60 * SEC);
    }

    #[test]
    fn a_missing_interval_or_retries_uses_dockers_default() {
        // 缺省 interval 30s、retries 3：10m + 90s 仍在 10m + 5m 的余量里。
        assert_eq!(limit(Some(10 * 60), None, None), 15 * 60 * SEC);
    }

    #[test]
    fn a_non_positive_retries_counts_as_one() {
        assert_eq!(limit(Some(10 * 60), Some(30), Some(0)), 15 * 60 * SEC);
    }
}
