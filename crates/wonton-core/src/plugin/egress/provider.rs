//! 代理插件的声明、地址和可用性（ADR-0039）。地址只认声明，容器名沿用实例的命名规则。

use std::time::Duration;

use bollard::models::HealthStatusEnum;
use wonton_protocol::{EgressFailure, EgressProblem, EgressProxyProvider};

use super::address::ProxyAddress;
use crate::l10n;
use crate::plugin::entry::Entry;
use crate::plugin::languages::Languages;
use crate::plugin::manager::{InstallError, PluginManager, PluginStatus};
use crate::plugin::manifest::{Manifest, PluginId};

pub(in crate::plugin) fn provides_proxy(manifest: &Manifest) -> bool {
    manifest
        .provides
        .as_ref()
        .is_some_and(|p| p.egress_proxy.is_some())
}

pub(in crate::plugin) struct Provider {
    /// 主人那边的语言下插件的名字。
    pub title: String,
    pub address: ProxyAddress,
    pub network: String,
    status: PluginStatus,
}

/// 状态没变时等这么久再看一眼容器：装的过程里 Docker 的状态自己会往前（或者镜像拉完），
/// 一下没看到运行中不代表装坏了。每次核对（预览、探测、安装）都会多等这一秒。
pub(super) const STATUS_RECHECK: Duration = Duration::from_secs(1);

/// 看一眼的结论：问题为空就是就绪；`status_waiting` 表示问题是“状态还没走到”来的，值得再看一眼。
struct Verdict {
    status_waiting: bool,
    problem: Option<EgressProblem>,
}

impl PluginManager {
    pub(in crate::plugin) fn egress_provider(
        &self,
        langs: &Languages,
    ) -> Result<Provider, InstallError> {
        let plugins = self.plugins.read().expect("插件表锁中毒");
        let (id, entry) = plugins
            .iter()
            .find(|(_, e)| e.provides_egress_proxy)
            .ok_or_else(|| {
                InstallError::NotInstalled(l10n::text(
                    langs,
                    "plugin-settings-egress-provider-missing",
                    &[],
                ))
            })?;
        let spec = entry
            .listed
            .as_ref()
            .and_then(|l| l.manifest.provides.as_ref())
            .and_then(|p| p.egress_proxy.as_ref())
            .ok_or_else(|| {
                InstallError::Storage(l10n::text(
                    langs,
                    "plugin-settings-egress-provider-manifest",
                    &[("id", id.to_string().into())],
                ))
            })?;
        let container = self.cfg.instance.container(&id.slug(), &spec.service);
        let address = ProxyAddress::parse(&format!("http://{container}:{}", spec.port), langs)
            .map_err(InstallError::Storage)?;
        Ok(Provider {
            title: langs.pick_owned(&entry.title),
            address,
            network: self.cfg.instance.network(&id.slug()),
            status: entry.status.clone(),
        })
    }

    pub(in crate::plugin) fn egress_provider_id(&self) -> Option<PluginId> {
        let plugins = self.plugins.read().expect("插件表锁中毒");
        plugins
            .iter()
            .find(|(_, entry)| entry.provides_egress_proxy)
            .map(|(id, _)| id.clone())
    }

    pub(super) fn egress_provider_view(&self, langs: &Languages) -> Option<EgressProxyProvider> {
        let plugins = self.plugins.read().expect("插件表锁中毒");
        plugins
            .iter()
            .find(|(_, e)| e.provides_egress_proxy)
            .map(|(id, entry)| EgressProxyProvider {
                id: id.to_string(),
                title: langs.pick_owned(&entry.title),
            })
    }

    pub(in crate::plugin) fn check_proxy_provider(
        &self,
        manifest: &Manifest,
        plugins: &std::collections::BTreeMap<PluginId, Entry>,
        langs: &Languages,
    ) -> Result<(), InstallError> {
        if provides_proxy(manifest)
            && let Some((id, _)) = plugins
                .iter()
                .find(|(id, entry)| **id != manifest.id && entry.provides_egress_proxy)
        {
            return Err(InstallError::Conflict(l10n::text(
                langs,
                "plugin-settings-egress-provider-conflict",
                &[("id", id.to_string().into())],
            )));
        }
        Ok(())
    }

    pub(super) async fn proxy_provider_problem(
        &self,
        langs: &Languages,
    ) -> Result<Option<EgressProblem>, InstallError> {
        let verdict = self
            .provider_verdict(&self.egress_provider(langs)?, langs)
            .await?;
        // 状态还停在“在装、在换版本”上：等一秒，状态没变就按真实状态再认一次。拉镜像要几分钟，
        // 这段时间容器查不到、也还没起，不能就此说它坏了。
        if verdict.status_waiting {
            tokio::time::sleep(STATUS_RECHECK).await;
            return self
                .provider_verdict(&self.egress_provider(langs)?, langs)
                .await
                .map(|v| v.problem);
        }
        Ok(verdict.problem)
    }

    async fn provider_verdict(
        &self,
        provider: &Provider,
        langs: &Languages,
    ) -> Result<Verdict, InstallError> {
        let container = provider.address.host.as_str();
        let mut verdict = Verdict {
            status_waiting: false,
            problem: None,
        };
        let waiting = |key: &str| EgressProblem {
            reason: EgressFailure::Waiting,
            message: l10n::text(
                langs,
                key,
                &[
                    ("title", provider.title.clone().into()),
                    ("container", container.into()),
                ],
            ),
        };
        match &provider.status {
            PluginStatus::Installing => {
                verdict.status_waiting = true;
                verdict.problem = Some(waiting("plugin-settings-egress-provider-installing"));
                return Ok(verdict);
            }
            PluginStatus::Upgrading => {
                verdict.status_waiting = true;
                verdict.problem = Some(waiting("plugin-settings-egress-provider-upgrading"));
                return Ok(verdict);
            }
            PluginStatus::Uninstalling => {
                verdict.problem = Some(waiting("plugin-settings-egress-provider-uninstalling"));
                return Ok(verdict);
            }
            _ => {}
        }
        let state = self
            .engine
            .inspect_container(container)
            .await?
            .and_then(|info| info.state);
        let running = state.as_ref().and_then(|s| s.running) == Some(true);
        let health = state
            .as_ref()
            .and_then(|s| s.health.as_ref())
            .and_then(|h| h.status);
        let starting = health == Some(HealthStatusEnum::STARTING);
        if starting {
            verdict.problem = Some(waiting("plugin-settings-egress-provider-starting"));
            return Ok(verdict);
        }
        // 容器不在跑也说就绪，就是把出口插件背后的容器当成活的：它得真在跑。
        if provider.status == PluginStatus::Running
            && running
            && health != Some(HealthStatusEnum::UNHEALTHY)
        {
            return Ok(verdict);
        }
        let Some(state) = state else {
            verdict.problem = Some(EgressProblem {
                reason: EgressFailure::ProxyUnreachable,
                message: l10n::text(
                    langs,
                    "plugin-settings-egress-provider-no-container",
                    &[
                        ("title", provider.title.clone().into()),
                        ("container", container.into()),
                    ],
                ),
            });
            return Ok(verdict);
        };
        let code = state
            .exit_code
            .map_or_else(|| "-".to_owned(), |code| code.to_string());
        let status = match state.status {
            Some(status) => format!("{status:?}").to_lowercase(),
            None => "unknown".to_owned(),
        };
        verdict.problem = Some(EgressProblem {
            reason: EgressFailure::ProxyUnreachable,
            message: l10n::text(
                langs,
                if health == Some(HealthStatusEnum::UNHEALTHY) {
                    "plugin-settings-egress-provider-unhealthy"
                } else {
                    "plugin-settings-egress-provider-stopped"
                },
                &[
                    ("title", provider.title.clone().into()),
                    ("container", container.into()),
                    ("status", status.into()),
                    ("code", code.into()),
                ],
            ),
        });
        Ok(verdict)
    }
}
