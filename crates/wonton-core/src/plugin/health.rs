//! 健康检查：等待容器达到 compose 依赖条件，安装完成前确认每个容器健康、网页可访问。

use std::time::Duration;

use bollard::models::{ContainerStateStatusEnum, HealthStatusEnum};
use tokio::time::{Instant, sleep};

use super::compose::DependCondition;
use super::docker::{Engine, EngineError};
use crate::http::HttpClient;

const POLL: Duration = Duration::from_millis(500);
/// 没有健康检查的容器至少要稳定运行这么久才算就绪，避免把“启动即崩溃”当成成功。
const STABLE_FOR: Duration = Duration::from_secs(3);
const LOG_LINES: usize = 40;

#[derive(Debug, thiserror::Error)]
pub enum HealthError {
    #[error(transparent)]
    Engine(#[from] EngineError),
    #[error("容器 {container} 在期限内没有就绪（最后状态：{state}）\n最近日志：\n{logs}")]
    Timeout {
        container: String,
        state: String,
        logs: String,
    },
    #[error("容器 {container} 健康检查失败\n最近日志：\n{logs}")]
    Unhealthy { container: String, logs: String },
    #[error("容器 {container} 已退出（退出码 {code}）\n最近日志：\n{logs}")]
    Exited {
        container: String,
        code: i64,
        logs: String,
    },
    #[error("容器 {0} 被要求 service_healthy，但没有配置健康检查")]
    NoHealthcheck(String),
    #[error("网页入口 {url} 在期限内没有响应：{last_error}")]
    WebUi { url: String, last_error: String },
}

/// 等待目标状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// compose `depends_on` 的条件。
    Depend(DependCondition),
    /// 安装完成前的最终检查：有健康检查就等到 healthy，否则要求稳定运行。
    Ready,
}

enum Observation {
    Done,
    Waiting(String),
}

pub async fn wait_for(
    engine: &Engine,
    container: &str,
    target: Target,
    deadline: Instant,
) -> Result<(), HealthError> {
    let mut running_since: Option<Instant> = None;
    loop {
        let state = match observe(engine, container, target, &mut running_since).await {
            Ok(Observation::Done) => return Ok(()),
            Ok(Observation::Waiting(state)) => state,
            Err(Failure::Unhealthy) => {
                let logs = engine.logs_tail(container, LOG_LINES).await;
                return Err(HealthError::Unhealthy {
                    container: container.to_owned(),
                    logs,
                });
            }
            Err(Failure::Exited(code)) => {
                let logs = engine.logs_tail(container, LOG_LINES).await;
                return Err(HealthError::Exited {
                    container: container.to_owned(),
                    code,
                    logs,
                });
            }
            Err(Failure::Other(e)) => return Err(e),
        };
        if Instant::now() >= deadline {
            let logs = engine.logs_tail(container, LOG_LINES).await;
            return Err(HealthError::Timeout {
                container: container.to_owned(),
                state,
                logs,
            });
        }
        sleep(POLL).await;
    }
}

enum Failure {
    Unhealthy,
    Exited(i64),
    Other(HealthError),
}

async fn observe(
    engine: &Engine,
    container: &str,
    target: Target,
    running_since: &mut Option<Instant>,
) -> Result<Observation, Failure> {
    let info = engine
        .inspect_container(container)
        .await
        .map_err(|e| Failure::Other(e.into()))?
        .ok_or_else(|| Failure::Other(EngineError::NotFound(format!("容器 {container}")).into()))?;
    let state = info.state.unwrap_or_default();
    let status = state.status.unwrap_or(ContainerStateStatusEnum::EMPTY);
    let health = state.health.and_then(|h| h.status);
    let has_check = matches!(
        health,
        Some(HealthStatusEnum::STARTING | HealthStatusEnum::HEALTHY | HealthStatusEnum::UNHEALTHY)
    );
    let exit_code = state.exit_code.unwrap_or_default();
    let label = format!(
        "{status:?}{}",
        health.map(|h| format!(" / {h:?}")).unwrap_or_default()
    );

    if target == Target::Depend(DependCondition::ServiceCompletedSuccessfully) {
        return match status {
            ContainerStateStatusEnum::EXITED if exit_code == 0 => Ok(Observation::Done),
            ContainerStateStatusEnum::EXITED | ContainerStateStatusEnum::DEAD => {
                Err(Failure::Exited(exit_code))
            }
            _ => Ok(Observation::Waiting(label)),
        };
    }
    if matches!(
        status,
        ContainerStateStatusEnum::EXITED | ContainerStateStatusEnum::DEAD
    ) {
        return Err(Failure::Exited(exit_code));
    }
    if health == Some(HealthStatusEnum::UNHEALTHY) {
        return Err(Failure::Unhealthy);
    }
    let running = status == ContainerStateStatusEnum::RUNNING;
    match target {
        Target::Depend(DependCondition::ServiceHealthy) if !has_check => Err(Failure::Other(
            HealthError::NoHealthcheck(container.to_owned()),
        )),
        Target::Depend(DependCondition::ServiceStarted) if running => Ok(Observation::Done),
        Target::Depend(DependCondition::ServiceHealthy) | Target::Ready if has_check => {
            if health == Some(HealthStatusEnum::HEALTHY) {
                Ok(Observation::Done)
            } else {
                Ok(Observation::Waiting(label))
            }
        }
        Target::Ready if running => {
            let since = *running_since.get_or_insert_with(Instant::now);
            if since.elapsed() >= STABLE_FOR {
                Ok(Observation::Done)
            } else {
                Ok(Observation::Waiting(label))
            }
        }
        _ => {
            *running_since = None;
            Ok(Observation::Waiting(label))
        }
    }
}

/// 反复请求网页入口，直到返回非 5xx 的响应。
pub async fn wait_webui(
    client: &HttpClient,
    url: &str,
    deadline: Instant,
) -> Result<(), HealthError> {
    loop {
        let last_error = match client.get_status(url, Duration::from_secs(5)).await {
            Ok(status) if status < 500 => return Ok(()),
            Ok(status) => format!("HTTP {status}"),
            Err(e) => e.to_string(),
        };
        if Instant::now() >= deadline {
            return Err(HealthError::WebUi {
                url: url.to_owned(),
                last_error,
            });
        }
        sleep(POLL).await;
    }
}
