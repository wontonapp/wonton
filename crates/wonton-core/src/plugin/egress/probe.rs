//! 探测（ADR-0023 的“代理连不上时怎么报错”）：插件的路径（经出口容器）和核心自己的路径（直接连代理）。上一条失败才用一次性容器
//! 查原因。
//!
//! 运行中坏了只显示在设置页和局域网状态页，插件不标异常：它可能根本没在用外网，状态标错反而误导。

use std::sync::Arc;
#[cfg(test)]
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bollard::models::{ContainerCreateBody, HostConfig};
use wonton_protocol::{EgressFailure, EgressProbe, EgressProblem};

use super::address::ProxyAddress;
use super::check::{self, TARGET, URL_VAR};
use super::provider::provides_proxy;
use crate::plugin::lan::probe::NOBODY_USER;
use crate::plugin::manager::{InstallError, PluginManager};
use crate::plugin::manifest::{Manifest, Permission};

/// 和 ADR-0019 的检测同一节奏。
pub const INTERVAL: Duration = crate::plugin::lan::INTERVAL;
/// 探测结果这么久以内算数：预览和安装不用再等一次。
const FRESH: Duration = Duration::from_secs(60);
/// 经 gost 等代理的超时：gost 默认 15 秒才回 503（实测）。
const VIA_RELAY_TIMEOUT: Duration = Duration::from_secs(20);

/// 怎么发探测。
#[derive(Clone, Default)]
pub enum Prober {
    #[cfg_attr(not(test), default)]
    Real,
    /// 测试用：结果由测试给出。
    #[cfg(test)]
    Fixed(Arc<Mutex<Scripted>>),
    /// 测试默认都通，不连网。
    #[cfg(test)]
    #[default]
    Fine,
}

/// 测试给的探测结果。
#[cfg(test)]
#[derive(Debug, Clone, Default)]
pub struct Scripted {
    /// 经出口容器访问外面失败的原因。
    pub via_relay: Option<String>,
    /// 核心直接连代理的结果。
    pub direct: Option<EgressProblem>,
}

impl Prober {
    async fn via_relay(&self, relay: &str) -> Result<(), String> {
        match self {
            Self::Real => check::fetch(relay, TARGET.url, VIA_RELAY_TIMEOUT).await,
            #[cfg(test)]
            Self::Fixed(script) => match script.lock().expect("探测脚本锁中毒").via_relay.clone()
            {
                Some(reason) => Err(reason),
                None => Ok(()),
            },
            #[cfg(test)]
            Self::Fine => Ok(()),
        }
    }

    async fn direct(&self, proxy: &ProxyAddress) -> Option<EgressProblem> {
        match self {
            Self::Real => check::check(proxy, TARGET).await,
            #[cfg(test)]
            Self::Fixed(script) => script.lock().expect("探测脚本锁中毒").direct.clone(),
            #[cfg(test)]
            Self::Fine => None,
        }
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or_default()
}

fn relay_failed(message: String) -> EgressProblem {
    EgressProblem {
        reason: EgressFailure::RelayFailed,
        message,
    }
}

/// 这个结果挡不挡安装。只看到一个网站不通，不等于插件要连的地方都不通，所以 `upstream_failed` 不挡。
pub(crate) fn blocks(problem: &EgressProblem) -> bool {
    problem.reason != EgressFailure::UpstreamFailed
}

pub(in crate::plugin) fn fresh(probe: &EgressProbe) -> bool {
    let age = now_ms().saturating_sub(probe.probed_at_ms);
    u128::from(age) < FRESH.as_millis()
}

impl PluginManager {
    /// 探测两条路径，记下结果，结论变了写日志。
    pub async fn probe_egress(&self) -> Result<(), InstallError> {
        let config = self.egress_config()?;
        let Some(proxy) = &config.url else {
            self.egress.record_probes(None, None);
            return Ok(());
        };
        let unavailable = if config.source == wonton_protocol::EgressProxySource::Plugin {
            self.proxy_provider_problem().await?
        } else {
            None
        };
        let (plugin, core) = match unavailable {
            Some(problem) => (Some(problem.clone()), Some(problem)),
            None => (
                self.probe_plugin_path(proxy).await?,
                self.egress.prober().direct(proxy).await,
            ),
        };
        let probed_at_ms = now_ms();
        if let Some(problem) = &plugin {
            tracing::warn!(reason = ?problem.reason, message = %problem.message, "插件经代理出不去");
        }
        self.egress.record_probes(
            Some(EgressProbe {
                probed_at_ms,
                problem: plugin,
            }),
            Some(EgressProbe {
                probed_at_ms,
                problem: core,
            }),
        );
        Ok(())
    }

    async fn probe_plugin_path(
        &self,
        proxy: &ProxyAddress,
    ) -> Result<Option<EgressProblem>, InstallError> {
        let name = self.cfg.instance.egress();
        // 每次都核对：出口容器可能被删了，核心换了容器后也要重新接上它的网络。
        match self.reconcile_relay().await {
            Err(e @ (InstallError::ImagePull(_) | InstallError::Docker(_))) => {
                return Ok(Some(relay_failed(e.to_string())));
            }
            result => result?,
        }
        let Some(address) = self.engine.endpoint_address(&name, &name).await? else {
            return Ok(Some(relay_failed("出口容器没有在运行".to_owned())));
        };
        let relay = format!("http://{address}:3128");
        let Err(detail) = self.egress.prober().via_relay(&relay).await else {
            return Ok(None);
        };
        let found = self.diagnose(proxy).await?;
        Ok(Some(found.unwrap_or_else(|| {
            relay_failed(format!(
                "直接连代理是通的，经出口容器不通：{detail}。查看容器 {name} 的日志"
            ))
        })))
    }

    /// 查原因：核心在容器里时，在出口网络上的一次性容器里查；直接跑在宿主上时就地查。查不了算出口容器的问题。
    pub(in crate::plugin) async fn diagnose(
        &self,
        proxy: &ProxyAddress,
    ) -> Result<Option<EgressProblem>, InstallError> {
        let config = self.egress_config()?;
        let network = if config.source == wonton_protocol::EgressProxySource::Plugin {
            self.egress_provider()?.network
        } else {
            self.cfg.instance.egress()
        };
        self.diagnose_at(proxy, &network).await
    }

    pub(super) async fn diagnose_at(
        &self,
        proxy: &ProxyAddress,
        network: &str,
    ) -> Result<Option<EgressProblem>, InstallError> {
        let Some(core) = &self.cfg.core_container else {
            return Ok(self.egress.prober().direct(proxy).await);
        };
        let Some(image) = self
            .engine
            .inspect_container(core)
            .await?
            .and_then(|c| c.image)
        else {
            return Ok(Some(relay_failed(format!("查不到核心容器 {core} 的镜像"))));
        };
        let body = ContainerCreateBody {
            image: Some(image),
            cmd: Some(vec![check::COMMAND.to_owned()]),
            env: Some(vec![format!("{URL_VAR}={}", proxy.text())]),
            user: Some(NOBODY_USER.to_owned()),
            labels: Some(self.cfg.instance.core_labels()),
            host_config: Some(HostConfig {
                network_mode: Some(network.to_owned()),
                cap_drop: Some(vec!["ALL".to_owned()]),
                security_opt: Some(vec!["no-new-privileges".to_owned()]),
                readonly_rootfs: Some(true),
                extra_hosts: Some(vec!["host.docker.internal:host-gateway".to_owned()]),
                ..Default::default()
            }),
            ..Default::default()
        };
        let name = self.cfg.instance.egress_check();
        let _diagnosing = self.egress.diagnosing.lock().await;
        let output = match self.engine.run_once(&name, body).await? {
            Ok(output) => output,
            Err(failure) => return Ok(Some(relay_failed(failure))),
        };
        match serde_json::from_slice::<Option<EgressProblem>>(&output.stdout) {
            Ok(found) => Ok(found),
            Err(_) => Ok(Some(relay_failed(format!(
                "查原因的容器以 {} 退出：{}",
                output.code,
                String::from_utf8_lossy(&output.stderr).trim()
            )))),
        }
    }

    /// 预览和安装出网插件时：结果不新鲜就再探一次，返回插件的路径上的问题。没配代理、不出网的没有。
    pub(in crate::plugin) async fn egress_problem_for(
        &self,
        manifest: &Manifest,
    ) -> Result<Option<EgressProblem>, InstallError> {
        if provides_proxy(manifest)
            || !manifest.permissions.contains(&Permission::NetworkEgress)
            || self.egress_config()?.url.is_none()
        {
            return Ok(None);
        }
        if self.egress_config()?.source == wonton_protocol::EgressProxySource::Plugin
            && let Some(problem) = self.proxy_provider_problem().await?
        {
            return Ok(Some(problem));
        }
        let latest = self.egress.plugin_path().filter(fresh);
        let latest = match latest {
            Some(latest) => latest,
            None => {
                self.probe_egress().await?;
                match self.egress.plugin_path() {
                    Some(latest) => latest,
                    None => return Ok(None),
                }
            }
        };
        Ok(latest.problem)
    }

    /// 配了代理、插件的路径不通时不让装。
    pub(in crate::plugin) async fn check_egress(
        &self,
        manifest: &Manifest,
    ) -> Result<(), InstallError> {
        match self.egress_problem_for(manifest).await? {
            Some(problem) if blocks(&problem) => Err(InstallError::EgressProxyUnavailable {
                reason: problem.reason,
                message: problem.message,
            }),
            _ => Ok(()),
        }
    }

    /// 插件的路径上最近的问题，局域网状态页显示。
    pub fn egress_problem(&self) -> Option<EgressProblem> {
        self.egress.plugin_path().and_then(|probe| probe.problem)
    }

    /// 核心启动后按设置核对一次，之后每 [`INTERVAL`] 探测一次。都在后台：拉 gost 的镜像、重启插件可能要等很久，不能拖慢核心启动。
    pub fn spawn_egress_monitor(self: &Arc<Self>) {
        let this = Arc::downgrade(self);
        tokio::spawn(async move {
            let mut ticks = tokio::time::interval(INTERVAL);
            let mut first = true;
            loop {
                ticks.tick().await;
                let Some(manager) = this.upgrade() else {
                    return;
                };
                if first && let Err(e) = manager.apply_egress().await {
                    tracing::warn!(error = %e, "核对出口容器失败");
                }
                first = false;
                if manager.egress_configured()
                    && let Err(e) = manager.probe_egress().await
                {
                    tracing::warn!(error = %e, "探测代理失败");
                }
            }
        });
    }
}
