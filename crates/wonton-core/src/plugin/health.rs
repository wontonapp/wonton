//! 健康检查：等待容器达到 compose 依赖条件，安装完成前确认每个容器健康、网页可访问。

use std::time::Duration;

use bollard::models::{ContainerStateStatusEnum, HealthStatusEnum};
use tokio::time::{Instant, sleep};

use super::compose::DependCondition;
use super::docker::{Engine, EngineError};
use super::oom::{OomLimit, oom_reason};
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

/// 容器达到目标时的情形。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Readiness {
    /// 健康检查通过。
    Healthy,
    /// 没有用到健康检查：稳定运行了一段时间、已经启动或以 0 退出。
    Running,
}

enum Observation {
    Done(Readiness),
    Waiting(String),
}

pub async fn wait_for(
    engine: &Engine,
    container: &str,
    target: Target,
    deadline: Instant,
) -> Result<Readiness, HealthError> {
    let mut running_since: Option<Instant> = None;
    loop {
        let state = match observe(engine, container, target, &mut running_since).await {
            Ok(Observation::Done(readiness)) => return Ok(readiness),
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
            ContainerStateStatusEnum::EXITED if exit_code == 0 => {
                Ok(Observation::Done(Readiness::Running))
            }
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
        Target::Depend(DependCondition::ServiceStarted) if running => {
            Ok(Observation::Done(Readiness::Running))
        }
        Target::Depend(DependCondition::ServiceHealthy) | Target::Ready if has_check => {
            if health == Some(HealthStatusEnum::HEALTHY) {
                Ok(Observation::Done(Readiness::Healthy))
            } else {
                Ok(Observation::Waiting(label))
            }
        }
        Target::Ready if running => {
            let since = *running_since.get_or_insert_with(Instant::now);
            if since.elapsed() >= STABLE_FOR {
                Ok(Observation::Done(Readiness::Running))
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

/// 核心启动时核对一个容器的结果：只看一眼，不等待。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Checked {
    /// 一次性服务已经以 0 退出（或者被重新运行着）。
    Completed,
    /// 长期运行，健康检查通过。
    Healthy,
    /// 长期运行，但没有健康检查，或者检查还没通过：只知道它在运行。
    Running,
    /// 常驻容器已退出或已创建但未启动。
    Stopped,
    /// 不在应有的状态，原因如实给出。
    Failed(String),
}

/// 核心重启后核对容器：一次性服务“已退出、退出码 0”是正常状态，长期运行的服务要在运行、健康检查不能失败。
pub async fn check_now(
    engine: &Engine,
    container: &str,
    service: &str,
    one_shot: bool,
) -> Result<Checked, EngineError> {
    let Some(info) = engine.inspect_container(container).await? else {
        return Ok(Checked::Failed(format!("容器 {container} 不存在")));
    };
    let state = info.state.unwrap_or_default();
    let status = state.status.unwrap_or(ContainerStateStatusEnum::EMPTY);
    let exit_code = state.exit_code.unwrap_or_default();
    let health = state.health.and_then(|h| h.status);
    let running = status == ContainerStateStatusEnum::RUNNING;
    let limit = OomLimit::of(info.host_config.as_ref());
    // 只杀了子进程时主进程可以自己以 0 退出，`OOMKilled` 也是 true：一次性服务这样算做完了。
    if state.oom_killed == Some(true) && !running && !(one_shot && exit_code == 0) {
        return Ok(Checked::Failed(oom_reason(service, limit)));
    }
    Ok(match (one_shot, status) {
        (true, ContainerStateStatusEnum::EXITED) if exit_code == 0 => Checked::Completed,
        (true, _) if running => Checked::Completed,
        (true, _) => Checked::Failed(format!(
            "一次性服务的容器 {container} 没有以 0 退出（{status:?}，退出码 {exit_code}）"
        )),
        (false, ContainerStateStatusEnum::EXITED | ContainerStateStatusEnum::CREATED) => {
            Checked::Stopped
        }
        (false, _) if !running => {
            Checked::Failed(format!("容器 {container} 不在运行（{status:?}）"))
        }
        (false, _) => match health {
            Some(HealthStatusEnum::HEALTHY) => Checked::Healthy,
            Some(HealthStatusEnum::UNHEALTHY) => {
                Checked::Failed(format!("容器 {container} 健康检查失败"))
            }
            _ => Checked::Running,
        },
    })
}

/// 反复请求网页入口，直到返回非 5xx 的响应。
pub async fn wait_webui(
    client: &HttpClient,
    endpoint: &super::upstreams::Endpoint,
    path: &str,
    deadline: Instant,
) -> Result<(), HealthError> {
    let url = endpoint.url(path);
    loop {
        let last_error =
            match super::upstreams::status(client, endpoint, path, Duration::from_secs(5)).await {
                Ok(status) if status < 500 => return Ok(()),
                Ok(status) => format!("HTTP {status}"),
                Err(e) => e.to_string(),
            };
        if Instant::now() >= deadline {
            return Err(HealthError::WebUi { url, last_error });
        }
        sleep(POLL).await;
    }
}
