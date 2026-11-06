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
/// 失败时带上最近几次探针的结果：Docker 至多存 5 条，3 次足够看出趋势。
const PROBES_KEPT: usize = 3;
/// 每条探针输出截到 1 KiB，防止刷屏。
const PROBE_OUTPUT_LIMIT: usize = 1024;

#[derive(Debug, thiserror::Error)]
pub enum HealthError {
    #[error(transparent)]
    Engine(#[from] EngineError),
    #[error("容器 {container} 在期限内没有就绪（最后状态：{state}）\n{probes}最近日志：\n{logs}")]
    Timeout {
        container: String,
        state: String,
        probes: String,
        logs: String,
    },
    #[error("容器 {container} 健康检查失败\n{probes}最近日志：\n{logs}")]
    Unhealthy {
        container: String,
        probes: String,
        logs: String,
    },
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
    /// 还没到目标：状态描述和最近几次探针的结果（超时报告用）。
    Waiting {
        state: String,
        probes: Vec<Probe>,
    },
}

pub async fn wait_for(
    engine: &Engine,
    container: &str,
    target: Target,
    deadline: Instant,
) -> Result<Readiness, HealthError> {
    let mut running_since: Option<Instant> = None;
    loop {
        let (state, probes) = match observe(engine, container, target, &mut running_since).await {
            Ok(Observation::Done(readiness)) => return Ok(readiness),
            Ok(Observation::Waiting { state, probes }) => (state, probes),
            Err(failure) => return Err(failure_error(engine, container, failure).await),
        };
        if Instant::now() >= deadline {
            let logs = engine.logs_tail(container, LOG_LINES).await;
            return Err(HealthError::Timeout {
                container: container.to_owned(),
                state,
                probes: probe_block(&probes),
                logs,
            });
        }
        sleep(POLL).await;
    }
}

/// 一起等一组 `depends_on`：每一轮把还没满足的都看一遍，任何一个失败就立即报它（容器名、退出码、日志），
/// 不再等其余依赖到超时。超时报第一个还没满足的。
pub(super) async fn wait_for_all(
    engine: &Engine,
    deps: &[(String, DependCondition)],
    deadline: Instant,
) -> Result<(), HealthError> {
    let mut pending: Vec<Pending> = deps
        .iter()
        .map(|(name, condition)| Pending {
            name,
            condition: *condition,
            state: String::new(),
            probes: Vec::new(),
        })
        .collect();
    while !pending.is_empty() {
        let mut index = 0;
        while index < pending.len() {
            let dep = &mut pending[index];
            let mut running_since = None;
            let target = Target::Depend(dep.condition);
            match observe(engine, dep.name, target, &mut running_since).await {
                Ok(Observation::Done(_)) => {
                    pending.remove(index);
                }
                Ok(Observation::Waiting { state, probes }) => {
                    dep.state = state;
                    dep.probes = probes;
                    index += 1;
                }
                Err(failure) => return Err(failure_error(engine, dep.name, failure).await),
            }
        }
        if pending.is_empty() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            let dep = pending.swap_remove(0);
            let logs = engine.logs_tail(dep.name, LOG_LINES).await;
            return Err(HealthError::Timeout {
                container: dep.name.to_owned(),
                state: dep.state,
                probes: probe_block(&dep.probes),
                logs,
            });
        }
        sleep(POLL).await;
    }
    Ok(())
}

/// 还没满足的依赖和它最近一次看到的状态、探针结果（超时报告用）。
struct Pending<'a> {
    name: &'a str,
    condition: DependCondition,
    state: String,
    probes: Vec<Probe>,
}

/// 失败变成报错：附上容器最近的日志，让人不用去查 Docker 就能看出原因。
async fn failure_error(engine: &Engine, container: &str, failure: Failure) -> HealthError {
    match failure {
        Failure::Unhealthy(probes) => HealthError::Unhealthy {
            container: container.to_owned(),
            probes: probe_block(&probes),
            logs: engine.logs_tail(container, LOG_LINES).await,
        },
        Failure::Exited(code) => HealthError::Exited {
            container: container.to_owned(),
            code,
            logs: engine.logs_tail(container, LOG_LINES).await,
        },
        Failure::Other(e) => e,
    }
}

enum Failure {
    Unhealthy(Vec<Probe>),
    Exited(i64),
    Other(HealthError),
}

/// 一次健康检查探针：退出码和输出在 Docker 的 `State.Health.Log[]` 里，是失败原因的出处。
struct Probe {
    exit_code: Option<i64>,
    output: String,
}

/// 探针结果排成一段，和日志尾巴并列。没有 `Health.Log` 的旧 Docker 返回空段，错误和以前一样。
fn probe_block(probes: &[Probe]) -> String {
    if probes.is_empty() {
        return String::new();
    }
    let mut block = String::from("最近探针：\n");
    for probe in probes {
        // Docker 的探针输出常以换行结尾：trim_end 再拼，每条一行。
        let output = probe.output.trim_end();
        match probe.exit_code {
            Some(code) => block.push_str(&format!("[退出码 {code}] {output}\n")),
            None => block.push_str(&format!("[退出码 ?] {output}\n")),
        }
    }
    block
}

/// 探针输出常带终端控制字符：先去掉完整的 CSI 转义序列（`\x1b[参数 中间字节 结尾字节`，如
/// `\x1b[31m`），只滤 ESC 会留下 `[31m` 这样的残渣；再滤掉其余控制字符（保留换行和制表），按字符
/// 边界截到 1 KiB。日志尾巴没有脱敏，这里同样只截断、不新写脱敏规则。
fn clean_output(raw: &str) -> String {
    let cleaned: String = strip_csi(raw)
        .chars()
        .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
        .collect();
    let mut end = cleaned.len().min(PROBE_OUTPUT_LIMIT);
    while !cleaned.is_char_boundary(end) {
        end -= 1;
    }
    cleaned[..end].to_owned()
}

/// 去掉 CSI 序列：ESC 后跟 `[`，参数字节（0x30–0x3F）和中间字节（0x20–0x2F），0x40–0x7E 结束。
/// 输出被截断在序列中间时同样整段去掉。
fn strip_csi(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            while chars
                .peek()
                .is_some_and(|b| ('\u{20}'..='\u{3f}').contains(b))
            {
                chars.next();
            }
            if chars
                .peek()
                .is_some_and(|b| ('\u{40}'..='\u{7e}').contains(b))
            {
                chars.next();
            }
        } else {
            out.push(c);
        }
    }
    out
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
    let probes: Vec<Probe> = state
        .health
        .as_ref()
        .and_then(|h| h.log.as_ref())
        .map(|log| {
            log.iter()
                .rev()
                .take(PROBES_KEPT)
                .rev()
                .map(|entry| Probe {
                    exit_code: entry.exit_code,
                    output: clean_output(entry.output.as_deref().unwrap_or_default()),
                })
                .collect()
        })
        .unwrap_or_default();
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
            _ => Ok(Observation::Waiting {
                state: label,
                probes,
            }),
        };
    }
    if matches!(
        status,
        ContainerStateStatusEnum::EXITED | ContainerStateStatusEnum::DEAD
    ) {
        return Err(Failure::Exited(exit_code));
    }
    if health == Some(HealthStatusEnum::UNHEALTHY) {
        return Err(Failure::Unhealthy(probes));
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
                Ok(Observation::Waiting {
                    state: label,
                    probes,
                })
            }
        }
        Target::Ready if running => {
            let since = *running_since.get_or_insert_with(Instant::now);
            if since.elapsed() >= STABLE_FOR {
                Ok(Observation::Done(Readiness::Running))
            } else {
                Ok(Observation::Waiting {
                    state: label,
                    probes,
                })
            }
        }
        _ => {
            *running_since = None;
            Ok(Observation::Waiting {
                state: label,
                probes,
            })
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
