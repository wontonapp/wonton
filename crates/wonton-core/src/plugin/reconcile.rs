//! 核心重启或巡检时核对已装的插件：容器、网络、OIDC 客户端和密钥是否还和记录一致。

use std::collections::BTreeSet;
use std::sync::Arc;

use super::docker::NetworkReach;
use super::health::{self, Checked};
use super::health_gate::Gate;
use super::languages::Languages;
use super::lifecycle::client_spec;
use super::manager::{Deployed, Health, InstallError, PluginManager};
use super::manifest::Manifest;
use super::package::PluginPackage;
use super::proxy_auth::ProxyAuth;
use super::secrets::{self, Secrets};
use super::store::InstalledRecord;
use crate::l10n;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ContainersState {
    Running(Health),
    Stopped,
}

/// 看一眼插件的每个容器得到的结果。
pub(super) struct Containers {
    state: ContainersState,
    /// 能用的服务：它的每个容器（Bot 服务的每份）都在运行、健康检查没失败。一次性服务不在里面。
    serving: BTreeSet<String>,
    /// 不健康的容器（或其他问题）的原因，按请求的语言写好，一个容器一条。
    failures: Vec<String>,
}

impl Containers {
    /// 现在能用的服务：网关、MCP 和模型按它决定异常时照不照常给路由。
    pub(super) fn serving(&self) -> &BTreeSet<String> {
        &self.serving
    }

    /// 都就绪时为空；有不健康的就把它们一起列在原因里。
    pub(super) fn problem(&self, langs: &Languages) -> Option<InstallError> {
        let (first, rest) = self.failures.split_first()?;
        if rest.is_empty() {
            return Some(InstallError::Unhealthy(first.clone()));
        }
        let failures: Vec<&str> = self.failures.iter().map(String::as_str).collect();
        Some(InstallError::Unhealthy(l10n::text(
            langs,
            "plugin-install-containers-unhealthy",
            &[("failures", l10n::list(langs, &failures).into())],
        )))
    }

    /// 都就绪时给容器状态；有不健康的就把它们一起列在原因里。
    pub(super) fn ready(&self, langs: &Languages) -> Result<ContainersState, InstallError> {
        match self.problem(langs) {
            Some(problem) => Err(problem),
            None => Ok(self.state),
        }
    }
}

impl PluginManager {
    /// 核心重启后核对插件：一次性服务已以 0 退出，长期运行的容器在运行、健康检查没有失败；核心在插件
    /// 网络里、OIDC 客户端已登记、插件密钥都在，并重新收存一次插件写下的密钥。`None` 表示常驻服务全部停止。
    ///
    /// 有容器没就绪不算核对失败：照样给出能用的服务、上游、代登录凭据和 MCP 地址，原因放进 `problem`，
    /// 由网关、MCP 和模型按服务如实回答（见 [`Deployed`]）。网络、relay、密钥、OIDC 这些地址和凭据本身
    /// 的问题仍然整体失败。核对里的漂移重建按 `gate` 等健康。
    pub(super) async fn reconcile(
        &self,
        package: &PluginPackage,
        record: &InstalledRecord,
        gate: Gate,
    ) -> Result<Option<Deployed>, InstallError> {
        let manifest = &package.manifest;
        let slug = manifest.id.slug();
        let langs = self.owner_languages();
        let network = self.cfg.instance.network(&slug);
        let expected = self.network_reach(manifest);
        let actual = self.engine.network_reach(&network).await?;
        if actual != Some(expected) {
            return Err(InstallError::Conflict(l10n::text(
                &langs,
                "plugin-install-network-mismatch",
                &[
                    ("network", network.into()),
                    ("actual", describe_reach(actual, &langs).into()),
                    ("expected", describe_reach(Some(expected), &langs).into()),
                ],
            )));
        }
        // 先接核心再看容器：靠核心的内部别名才起得来的插件，核心不接回去就一直崩，先看容器的话两边互相等。
        if let Some(core) = &self.cfg.core_container {
            self.attach_core(&network, core, manifest).await?;
        }
        // 渲染和容器上的摘要对不上（核心升级换了注入的值，或旧容器没有摘要）就照安装的路径重建。全停着的
        // 插件不在这里动，留给启动流程（见 start.rs）。
        self.recreate_on_drift(package, record, &langs, false, gate)
            .await?;
        let checked = self.check_containers(package, &langs).await?;
        let health = match checked.state {
            ContainersState::Stopped => return Ok(None),
            ContainersState::Running(health) => health,
        };
        let problem = checked.problem(&langs).map(|problem| problem.to_string());
        self.ensure_relays(manifest, record).await?;
        self.check_bot(manifest).await?;
        let secrets = self.check_secrets(manifest).await?;
        let generated = secrets::generated_values(manifest, &secrets, &self.owner_languages())
            .map_err(InstallError::Storage)?;
        let proxy =
            ProxyAuth::resolve(manifest, &generated, &langs).map_err(InstallError::Storage)?;
        if let Some(secret) = &record.oidc_client_secret {
            let ctx = self
                .install_context(
                    manifest,
                    Some(secret),
                    generated,
                    &record.effective_permissions(manifest),
                    &langs,
                )
                .await?;
            self.hydra(&langs)?
                .upsert_client(&client_spec(manifest, &ctx, secret))
                .await?;
        }
        Ok(Some(Deployed {
            upstream: self.webui_upstream(manifest, &network).await?,
            serving: checked.serving().clone(),
            problem,
            health,
            models: self.models_upstream(manifest, &network).await?,
            mcp: self.mcp_upstream(manifest, &network, &secrets).await?,
            proxy: proxy.map(Arc::new),
            firewalled: record.firewalled,
        }))
    }

    /// 跟着重建之后核对：重建刚按 `start_period` 等过健康，这里再漂移重建也按它等。
    pub(super) async fn reconcile_running(
        &self,
        package: &PluginPackage,
        record: &InstalledRecord,
    ) -> Result<Deployed, InstallError> {
        match self.reconcile(package, record, Gate::StartPeriod).await? {
            Some(deployed) => Ok(deployed),
            None => Err(InstallError::Unhealthy(l10n::text(
                &self.owner_languages(),
                "plugin-install-resident-stopped",
                &[],
            ))),
        }
    }

    /// 看一眼插件的每个容器，不等待。不因为第一个不健康的容器就停下，把每个都看一遍。
    pub(super) async fn check_containers(
        &self,
        package: &PluginPackage,
        langs: &Languages,
    ) -> Result<Containers, InstallError> {
        let slug = package.manifest.id.slug();
        let one_shot = package.one_shot_services();
        let bot_services = super::bot_services::bot_services(&package.manifest);
        let bots = self.bot_names(&package.manifest.id);
        let mut running = 0;
        let mut healthy = true;
        let mut stopped = Vec::new();
        let mut failures = Vec::new();
        let mut served = BTreeSet::new();
        // Bot 服务按 Bot 一组容器，每个都核对；全插件一份的服务照旧一个。
        let instances = |service: &str| -> Vec<String> {
            if bot_services.contains(service) && !bots.is_empty() {
                bots.iter()
                    .map(|bot| self.cfg.instance.bot_container(&slug, service, bot))
                    .collect()
            } else {
                vec![self.cfg.instance.container(&slug, service)]
            }
        };
        for service in package.service_names() {
            // 这个服务能不能用：它的每一份容器都在运行、健康检查没失败。
            let mut serving = true;
            for name in instances(service) {
                let checked = health::check_now(
                    &self.engine,
                    &name,
                    service,
                    one_shot.contains(service),
                    langs,
                )
                .await?;
                match checked {
                    Checked::Completed => {}
                    Checked::Healthy => running += 1,
                    Checked::Running => {
                        running += 1;
                        healthy = false;
                    }
                    Checked::Stopped => {
                        serving = false;
                        stopped.push(name);
                    }
                    Checked::Failed(reason) => {
                        serving = false;
                        failures.push(reason);
                    }
                }
            }
            if serving && !one_shot.contains(service) {
                served.insert(service.to_owned());
            }
        }
        let state = ContainersState::Running(if healthy {
            Health::Healthy
        } else {
            Health::Unknown
        });
        if stopped.is_empty() {
            return Ok(Containers {
                state,
                serving: served,
                failures,
            });
        }
        if running == 0 && failures.is_empty() {
            return Ok(Containers {
                state: ContainersState::Stopped,
                serving: served,
                failures,
            });
        }
        if running > 0 {
            failures.push(l10n::text(
                langs,
                "plugin-install-resident-partial",
                &[("containers", l10n::list(langs, &stopped).into())],
            ));
        }
        Ok(Containers {
            state,
            serving: served,
            failures,
        })
    }

    /// 核对插件密钥：生成的都还在，插件写下的重新收存一次，变了就更新。返回全量密钥（generate 和
    /// collect）：generate 项的文本值经 [`secrets::generated_values`] 取，MCP 头要引用 collect 项。
    pub(super) async fn check_secrets(&self, manifest: &Manifest) -> Result<Secrets, InstallError> {
        if manifest.secrets.is_empty() {
            return Ok(Secrets::new());
        }
        let store = self.store.secrets();
        let mut secrets = store.load(&manifest.id)?.unwrap_or_default();
        secrets::generated_values(manifest, &secrets, &self.owner_languages())
            .map_err(InstallError::Storage)?;
        if secrets::collect(
            &self.engine,
            &self.cfg.instance,
            manifest,
            &self.bot_names(&manifest.id),
            &mut secrets,
            &self.owner_languages(),
        )
        .await?
        {
            store.save(&manifest.id, &secrets)?;
        }
        Ok(secrets)
    }
}

fn describe_reach(reach: Option<NetworkReach>, langs: &Languages) -> String {
    match reach {
        None => l10n::text(langs, "plugin-install-reach-none", &[]),
        Some(NetworkReach::Egress) => l10n::text(langs, "plugin-install-reach-egress", &[]),
        Some(NetworkReach::Internal) => l10n::text(langs, "plugin-install-reach-internal", &[]),
        Some(NetworkReach::Isolated) => l10n::text(langs, "plugin-install-reach-isolated", &[]),
    }
}
