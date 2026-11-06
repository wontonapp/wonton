//! 插件健康探测（规范“插件健康探测”）：申请了 `plugins.health` 的插件经 `health.wonton.internal` 读所有
//! 插件的状态；详情接口再让核心当场探测一次目标插件的网页入口。只读：不返回日志、配置和密钥，不触发任何操作。

use std::time::Duration;

use serde::Serialize;

use super::entry::Entry;
use super::manager::PluginManager;
use super::manifest::PluginId;
use super::upstreams::{self, Endpoint, PROBE_TIMEOUT};
use crate::http::HttpError;

/// 一次网页探测的结果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WebuiProbe {
    pub reachable: bool,
    /// 探测到了 HTTP 响应时是状态码（包括应用自己回的错误响应）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub http_status: Option<u16>,
    /// 实际发起过请求时是耗时；没能发起（不知道入口地址）时没有这个字段。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u64>,
    /// 失败原因：`unreachable` 连不上，`timeout` 超时，`upstream_error` 上游回了 5xx。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_kind: Option<&'static str>,
}

/// 一个插件给 `plugins.health` 的状态。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HealthReport {
    pub id: PluginId,
    pub title: super::manifest::LocalizedText,
    /// 核心已知的状态，取值和插件状态一致：`running`、`stopped`、`degraded`、`installing`、
    /// `uninstalling`、`upgrading`。以后核心新增的状态不在这里的，按 `other` 对待（规范“插件健康探测”）。
    pub state: &'static str,
    /// 核心最近一次核对状态的时间（Unix 秒）。
    pub checked_at: u64,
    /// 详情接口才有：核心这一刻探测目标网页入口的结果。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub probe: Option<WebuiProbe>,
    /// `probe` 为空的原因。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub probe_reason: Option<&'static str>,
}

const NO_WEBUI: &str = "这个插件没有声明网页入口";
const NO_MANIFEST: &str = "这个插件的状态不完整（清单读不出来），没有已知的网页入口";

/// 详情接口要做的探测：目标网页入口的地址和路径，或者探测不了的原因。
enum Target {
    Attempt(Option<Endpoint>, String, String),
    Skip(&'static str),
}

impl PluginManager {
    /// 所有已装插件的状态（列表接口），不探测。
    pub fn health_reports(&self) -> Vec<HealthReport> {
        let plugins = self.plugins.read().expect("插件表锁中毒");
        plugins
            .iter()
            .map(|(id, entry)| report(id, entry, None))
            .collect()
    }

    /// 一个插件的状态加当场探测（详情接口）。没装的是 `None`。
    pub async fn health_report(&self, id: &PluginId) -> Option<HealthReport> {
        let (mut base, target) = {
            let plugins = self.plugins.read().expect("插件表锁中毒");
            let entry = plugins.get(id)?;
            let webui = entry
                .listed
                .as_ref()
                .and_then(|listed| listed.manifest.webui.as_ref());
            let target = match webui {
                Some(webui) => Target::Attempt(
                    entry.upstream.clone(),
                    webui.path.clone(),
                    self.cfg.public.plugin_host(&id.slug()),
                ),
                None if entry.listed.is_none() => Target::Skip(NO_MANIFEST),
                None => Target::Skip(NO_WEBUI),
            };
            (report(id, entry, None), target)
        };
        match target {
            Target::Skip(reason) => base.probe_reason = Some(reason),
            Target::Attempt(endpoint, path, host) => {
                base.probe = Some(match endpoint {
                    Some(endpoint) => {
                        let (elapsed, outcome) =
                            upstreams::probe(&self.http, &endpoint, &path, &host, PROBE_TIMEOUT)
                                .await;
                        outcome_of(Some(elapsed), outcome)
                    }
                    // 没有已知的入口地址（比如插件还没部署到那一步）：连不上，也没发起过请求。
                    None => WebuiProbe {
                        reachable: false,
                        http_status: None,
                        latency_ms: None,
                        error_kind: Some("unreachable"),
                    },
                });
            }
        }
        Some(base)
    }
}

/// 不带探测的状态行，列表和详情共用。
fn report(id: &PluginId, entry: &Entry, probe: Option<WebuiProbe>) -> HealthReport {
    HealthReport {
        id: id.clone(),
        title: entry.title.clone(),
        state: entry.status.state_name(),
        checked_at: entry.checked_at,
        probe,
        probe_reason: None,
    }
}

/// 探测结果换成接口的形状：5xx 算“上游报错”，别的状态码都算打得开（应用自己决定登录或拒绝）。
fn outcome_of(elapsed: Option<Duration>, outcome: Result<u16, HttpError>) -> WebuiProbe {
    let latency_ms = elapsed.map(|d| d.as_millis() as u64);
    match outcome {
        Ok(status) if status < 500 => WebuiProbe {
            reachable: true,
            http_status: Some(status),
            latency_ms,
            error_kind: None,
        },
        Ok(status) => WebuiProbe {
            reachable: false,
            http_status: Some(status),
            latency_ms,
            error_kind: Some("upstream_error"),
        },
        Err(HttpError::Timeout(_) | HttpError::ResponseHeadersTimeout(_)) => WebuiProbe {
            reachable: false,
            http_status: None,
            latency_ms,
            error_kind: Some("timeout"),
        },
        Err(_) => WebuiProbe {
            reachable: false,
            http_status: None,
            latency_ms,
            error_kind: Some("unreachable"),
        },
    }
}
