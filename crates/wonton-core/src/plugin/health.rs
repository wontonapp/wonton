//! 健康检查：等待容器达到 compose 依赖条件，安装完成前确认每个容器健康、网页可访问。

use std::time::Duration;

use bollard::models::{ContainerStateStatusEnum, HealthStatusEnum};
use tokio::time::{Instant, sleep};

use super::compose::DependCondition;
use super::docker::{Engine, EngineError};
use super::languages::Languages;
use super::oom::{OomLimit, oom_reason};
use crate::http::{HttpClient, HttpError};
use crate::l10n;

const POLL: Duration = Duration::from_millis(500);
/// 没有健康检查的容器至少要稳定运行这么久才算就绪，避免把“启动即崩溃”当成成功。
const STABLE_FOR: Duration = Duration::from_secs(3);
/// 失败时带上容器最后这么多行日志：只留几十行时，慢启动的应用真正的原因（第一行 FATAL）已经被截掉了。
/// 字节上限在 [`Engine::logs_tail`]。
const LOG_LINES: usize = 200;
/// 失败时带上最近几次探针的结果：Docker 至多存 5 条，3 次足够看出趋势。
const PROBES_KEPT: usize = 3;
/// 每条探针输出截到 1 KiB，防止刷屏。
const PROBE_OUTPUT_LIMIT: usize = 1024;

#[derive(Debug, thiserror::Error)]
pub enum HealthError {
    #[error(transparent)]
    Engine(#[from] EngineError),
    /// 给 App 看的整句，构造点按当次语言写好（ADR-0043）。
    #[error("{0}")]
    Timeout(String),
    #[error("{0}")]
    Unhealthy(String),
    #[error("{0}")]
    Exited(String),
    #[error("{0}")]
    NoHealthcheck(String),
    #[error("{0}")]
    WebUi(String),
}

/// 清单 `webui.health`：服务自己答健康的方式。镜像里没有 shell 时写不出 compose healthcheck，
/// 由核心按这个路径探测（和装完时的网页探测同一个地址和时限）。
pub struct WebuiHealth {
    pub endpoint: super::upstreams::Endpoint,
    pub path: String,
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

pub(super) async fn wait_for(
    engine: &Engine,
    http: &HttpClient,
    container: &str,
    target: Target,
    webui_health: Option<&WebuiHealth>,
    deadline: Instant,
    langs: &Languages,
) -> Result<Readiness, HealthError> {
    let mut running_since: Option<Instant> = None;
    loop {
        let (state, probes) = match observe(
            engine,
            http,
            container,
            target,
            webui_health,
            &mut running_since,
            langs,
        )
        .await
        {
            Ok(Observation::Done(readiness)) => return Ok(readiness),
            Ok(Observation::Waiting { state, probes }) => (state, probes),
            Err(failure) => return Err(failure_error(engine, container, failure, langs).await),
        };
        if Instant::now() >= deadline {
            let logs = traced(container, engine.logs_tail(container, LOG_LINES).await);
            return Err(HealthError::Timeout(with_log_tail(
                crate::l10n::text(
                    langs,
                    "plugin-settings-health-timeout",
                    &[("container", container.into()), ("state", state.into())],
                ),
                probe_block(&probes, langs),
                &logs,
                langs,
            )));
        }
        sleep(POLL).await;
    }
}

/// 一起等一组 `depends_on`：每一轮把还没满足的都看一遍，任何一个失败就立即报它（容器名、退出码、日志），
/// 不再等其余依赖到超时。超时报第一个还没满足的。
pub(super) async fn wait_for_all(
    engine: &Engine,
    http: &HttpClient,
    deps: &[(String, DependCondition)],
    deadline: Instant,
    langs: &Languages,
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
            match observe(
                engine,
                http,
                dep.name,
                target,
                None,
                &mut running_since,
                langs,
            )
            .await
            {
                Ok(Observation::Done(_)) => {
                    pending.remove(index);
                }
                Ok(Observation::Waiting { state, probes }) => {
                    dep.state = state;
                    dep.probes = probes;
                    index += 1;
                }
                Err(failure) => return Err(failure_error(engine, dep.name, failure, langs).await),
            }
        }
        if pending.is_empty() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            let dep = pending.swap_remove(0);
            let logs = traced(dep.name, engine.logs_tail(dep.name, LOG_LINES).await);
            return Err(HealthError::Timeout(with_log_tail(
                crate::l10n::text(
                    langs,
                    "plugin-settings-health-timeout",
                    &[
                        ("container", dep.name.into()),
                        ("state", dep.state.clone().into()),
                    ],
                ),
                probe_block(&dep.probes, langs),
                &logs,
                langs,
            )));
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
async fn failure_error(
    engine: &Engine,
    container: &str,
    failure: Failure,
    langs: &Languages,
) -> HealthError {
    match failure {
        Failure::Unhealthy(probes) => {
            let logs = traced(container, engine.logs_tail(container, LOG_LINES).await);
            HealthError::Unhealthy(with_log_tail(
                crate::l10n::text(
                    langs,
                    "plugin-settings-health-unhealthy",
                    &[("container", container.into())],
                ),
                probe_block(&probes, langs),
                &logs,
                langs,
            ))
        }
        Failure::Exited(code) => {
            let logs = traced(container, engine.logs_tail(container, LOG_LINES).await);
            HealthError::Exited(with_log_tail(
                crate::l10n::text(
                    langs,
                    "plugin-settings-health-exited",
                    &[("container", container.into()), ("code", code.into())],
                ),
                String::new(),
                &logs,
                langs,
            ))
        }
        Failure::Other(e) => e,
    }
}

/// 日志同时进核心日志：安装失败后容器会被回滚删掉，只看得到错误的人不至于连日志都没了。
fn traced(container: &str, logs: String) -> String {
    if !logs.is_empty() {
        tracing::warn!(%container, logs = %logs, "容器失败时的日志");
    }
    logs
}

/// 错误正文加上日志尾巴一段：没有探针就不留空行，和以前的排版一致。
fn with_log_tail(head: String, probes: String, logs: &str, langs: &Languages) -> String {
    let tail = crate::l10n::text(langs, "plugin-settings-health-log-tail", &[]);
    if probes.is_empty() {
        format!("{head}\n{tail}\n{logs}")
    } else {
        format!("{head}\n{probes}{tail}\n{logs}")
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

/// 最近 `keep` 条探针，最旧的在前。没有 `Health.Log` 的旧 Docker 为空。
fn recent_probes(state: &bollard::models::ContainerState, keep: usize) -> Vec<Probe> {
    let Some(log) = state.health.as_ref().and_then(|h| h.log.as_ref()) else {
        return Vec::new();
    };
    log.iter()
        .rev()
        .take(keep)
        .rev()
        .map(|entry| Probe {
            exit_code: entry.exit_code,
            output: clean_output(entry.output.as_deref().unwrap_or_default()),
        })
        .collect()
}

/// 健康检查失败的原因：带上最后一条探针的退出码和输出，让人不用去查 Docker 就能看出原因。
fn unhealthy_reason(container: &str, probe: Option<Probe>, langs: &Languages) -> String {
    let name = ("container", container.into());
    match probe {
        Some(Probe {
            exit_code: Some(code),
            output,
        }) => l10n::text(
            langs,
            "plugin-install-container-unhealthy",
            &[
                name,
                ("code", code.to_string().into()),
                ("output", output.into()),
            ],
        ),
        Some(Probe {
            exit_code: None,
            output,
        }) => l10n::text(
            langs,
            "plugin-install-container-unhealthy-nocode",
            &[name, ("output", output.into())],
        ),
        None => l10n::text(langs, "plugin-install-container-unhealthy-noprobe", &[name]),
    }
}

/// 探针结果排成一段，和日志尾巴并列。没有 `Health.Log` 的旧 Docker 返回空段，错误和以前一样。
fn probe_block(probes: &[Probe], langs: &Languages) -> String {
    if probes.is_empty() {
        return String::new();
    }
    let mut block = format!(
        "{}\n",
        crate::l10n::text(langs, "plugin-settings-health-probes", &[])
    );
    for probe in probes {
        // Docker 的探针输出常以换行结尾：trim_end 再拼，每条一行。
        let output = probe.output.trim_end();
        let line = match probe.exit_code {
            Some(code) => crate::l10n::text(
                langs,
                "plugin-settings-health-probe-line",
                &[("code", code.into()), ("output", output.into())],
            ),
            None => crate::l10n::text(
                langs,
                "plugin-settings-health-probe-line-unknown",
                &[("output", output.into())],
            ),
        };
        block.push_str(&line);
        block.push('\n');
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
    http: &HttpClient,
    container: &str,
    target: Target,
    webui_health: Option<&WebuiHealth>,
    running_since: &mut Option<Instant>,
    langs: &Languages,
) -> Result<Observation, Failure> {
    let info = engine
        .inspect_container(container)
        .await
        .map_err(|e| Failure::Other(e.into()))?
        .ok_or_else(|| {
            Failure::Other(
                EngineError::NotFound(crate::l10n::text(
                    langs,
                    "plugin-settings-health-container-missing",
                    &[("container", container.into())],
                ))
                .into(),
            )
        })?;
    let state = info.state.unwrap_or_default();
    let status = state.status.unwrap_or(ContainerStateStatusEnum::EMPTY);
    let probes = recent_probes(&state, PROBES_KEPT);
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
    // 只有在跑的网页容器才值得探测：别的状态由下面的判定先行处理。
    let webui = match webui_health {
        Some(webui) if target == Target::Ready && status == ContainerStateStatusEnum::RUNNING => {
            let outcome = super::upstreams::status(
                http,
                &webui.endpoint,
                &webui.path,
                super::upstreams::PROBE_TIMEOUT,
            )
            .await;
            Some((format!("GET {}", webui.path), outcome))
        }
        _ => None,
    };

    verdict(
        Snapshot {
            status,
            health,
            has_check,
            exit_code,
            label,
            probes,
            webui,
        },
        container,
        target,
        running_since,
        langs,
    )
}
/// 容器这一次读到的快照，够不够 `target` 由 [`verdict`] 判。
struct Snapshot {
    status: ContainerStateStatusEnum,
    health: Option<HealthStatusEnum>,
    has_check: bool,
    exit_code: i64,
    label: String,
    probes: Vec<Probe>,
    /// 清单 `webui.health` 的请求（`GET <路径>`）和结果，没有探测时为空。
    webui: Option<(String, Result<u16, HttpError>)>,
}

/// 读到的状态够不够 `target`：不够就继续等，够不了就给出失败原因。
fn verdict(
    snapshot: Snapshot,
    container: &str,
    target: Target,
    running_since: &mut Option<Instant>,
    langs: &Languages,
) -> Result<Observation, Failure> {
    let Snapshot {
        status,
        health,
        has_check,
        exit_code,
        label,
        probes,
        webui,
    } = snapshot;
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
    // 声明了 `webui.health`：以代探测为准，不管容器自己有没有健康检查。
    if let (Target::Ready, Some((request, outcome))) = (target, webui) {
        return match outcome {
            // 2xx/3xx 是健康；其余状态码等于应用自己说没起好，算上游出错。
            Ok(200..=399) => Ok(Observation::Done(Readiness::Healthy)),
            Ok(code) => Ok(Observation::Waiting {
                state: format!("{label} / {request} → HTTP {code}"),
                probes,
            }),
            Err(e) => Ok(Observation::Waiting {
                state: format!("{label} / {request} → {e}"),
                probes,
            }),
        };
    }
    match target {
        Target::Depend(DependCondition::ServiceHealthy) if !has_check => Err(Failure::Other(
            HealthError::NoHealthcheck(crate::l10n::text(
                langs,
                "plugin-settings-health-no-healthcheck",
                &[("container", container.into())],
            )),
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
    /// 容器不在：Docker 数据丢了、被手动删了，或者还没建出来。原因如实给出。
    Missing(String),
    /// 不在应有的状态，原因如实给出。
    Failed(String),
}

/// 核心重启后核对容器：一次性服务“已退出、退出码 0”是正常状态，长期运行的服务要在运行、健康检查不能失败。
pub async fn check_now(
    engine: &Engine,
    container: &str,
    service: &str,
    one_shot: bool,
    langs: &Languages,
) -> Result<Checked, EngineError> {
    let Some(info) = engine.inspect_container(container).await? else {
        return Ok(Checked::Missing(l10n::text(
            langs,
            "plugin-install-container-missing",
            &[("name", container.into())],
        )));
    };
    let state = info.state.unwrap_or_default();
    let status = state.status.unwrap_or(ContainerStateStatusEnum::EMPTY);
    let exit_code = state.exit_code.unwrap_or_default();
    let last_probe = recent_probes(&state, 1).pop();
    let health = state.health.and_then(|h| h.status);
    let running = status == ContainerStateStatusEnum::RUNNING;
    let limit = OomLimit::of(info.host_config.as_ref());
    // 只杀了子进程时主进程可以自己以 0 退出，`OOMKilled` 也是 true：一次性服务这样算做完了。
    if state.oom_killed == Some(true) && !running && !(one_shot && exit_code == 0) {
        return Ok(Checked::Failed(oom_reason(service, limit, langs)));
    }
    Ok(match (one_shot, status) {
        (true, ContainerStateStatusEnum::EXITED) if exit_code == 0 => Checked::Completed,
        (true, _) if running => Checked::Completed,
        (true, _) => Checked::Failed(crate::l10n::text(
            langs,
            "plugin-install-one-shot-failed",
            &[
                ("container", container.into()),
                ("status", format!("{status:?}").into()),
                ("code", exit_code.to_string().into()),
            ],
        )),
        (false, ContainerStateStatusEnum::EXITED | ContainerStateStatusEnum::CREATED) => {
            Checked::Stopped
        }
        (false, _) if !running => Checked::Failed(crate::l10n::text(
            langs,
            "plugin-install-container-not-running",
            &[
                ("container", container.into()),
                ("status", format!("{status:?}").into()),
            ],
        )),
        (false, _) => match health {
            Some(HealthStatusEnum::HEALTHY) => Checked::Healthy,
            Some(HealthStatusEnum::UNHEALTHY) => {
                Checked::Failed(unhealthy_reason(container, last_probe, langs))
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
    langs: &Languages,
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
            return Err(HealthError::WebUi(crate::l10n::text(
                langs,
                "plugin-settings-health-webui-timeout",
                &[("url", url.into()), ("last_error", last_error.into())],
            )));
        }
        sleep(POLL).await;
    }
}
