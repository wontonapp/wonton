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
    /// 不健康的容器（或其他问题）的原因，按请求的语言写好，一个容器一条。容器不在不算这里的问题：
    /// 全都不在时插件算停着，启动时照安装的路径重建（见 start.rs）。
    failures: Vec<String>,
    /// 不在的容器的原因，一条一个。插件还在跑时它们也进 `failures`（插件没死，但少了服务）；
    /// 启动看到它们就先照安装的路径重建，不看别的核对结果（见 start.rs）。
    missing: Vec<String>,
}

/// 核对结果的归类：正在跑、停着、缺容器和不健康混在一起时，按“是不是真问题”分成状态、能用的服务和原因。
fn classify(
    running: usize,
    healthy: bool,
    stopped: Vec<String>,
    missing: Vec<String>,
    failures: Vec<String>,
    served: BTreeSet<String>,
    langs: &Languages,
) -> Containers {
    // 容器不在不算停止以外的问题：常驻容器一个都没在跑、也没有真问题时（全不在的、和停着的混着的
    // 都算）插件算停着，不是异常——启动按已安装记录照安装的路径重建（issue #126），App 的启动按钮
    // 照常出现。只有它还缺在运行中间时才如实进原因：插件没死，但少了服务。
    let mut failures = failures;
    if running > 0 {
        failures.extend(missing.iter().cloned());
    }
    let state = if running > 0 || (stopped.is_empty() && missing.is_empty()) {
        // 有在跑的容器，或没有常驻容器可看（只有一次性服务）：照原有的分类。
        ContainersState::Running(if healthy {
            Health::Healthy
        } else {
            Health::Unknown
        })
    } else if failures.is_empty() {
        ContainersState::Stopped
    } else {
        // 没有在跑的容器，却有真问题（不健康、OOM、一次性服务失败）：照旧按异常核对报。
        ContainersState::Running(Health::Unknown)
    };
    if running > 0 && !stopped.is_empty() {
        failures.push(l10n::text(
            langs,
            "plugin-install-resident-partial",
            &[("containers", l10n::list(langs, &stopped).into())],
        ));
    }
    Containers {
        state,
        serving: served,
        failures,
        missing,
    }
}

impl Containers {
    /// 现在能用的服务：网关、MCP 和模型按它决定异常时照不照常给路由。
    pub(super) fn serving(&self) -> &BTreeSet<String> {
        &self.serving
    }

    /// 有容器不在：启动时不看别的核对结果，先按安装的路径重建（见 start.rs）。
    pub(super) fn has_missing(&self) -> bool {
        !self.missing.is_empty()
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
    /// 网络缺了就照安装规格重建，返回是否重建过；网络在但地址段对不上就整体报错——那不是漂移，是配错。
    async fn ensure_network_reach(&self, manifest: &Manifest) -> Result<bool, InstallError> {
        let slug = manifest.id.slug();
        let langs = self.owner_languages();
        let network = self.cfg.instance.network(&slug);
        let expected = self.network_reach(manifest);
        // 重建分支的 future 大（含建网络、起容器的整条安装路径），Box::pin 切断栈深度（见 #434）。
        let network_recreated = Box::pin(self.recreate_missing_network(manifest)).await?;
        // 刚重建的网络就是按 expected 建的，不用再问一遍 Docker。
        let actual = if network_recreated {
            Some(expected)
        } else {
            self.engine.network_reach(&network).await?
        };
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
        Ok(network_recreated)
    }

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
        if self.explicitly_stopped(&record.id)? {
            return Ok(None);
        }
        let manifest = &package.manifest;
        let slug = manifest.id.slug();
        let langs = self.owner_languages();
        self.ensure_bot_web_registration(manifest).await?;
        let network_recreated = self.ensure_network_reach(manifest).await?;
        let network = self.cfg.instance.network(&slug);
        // 先接核心再看容器：靠核心的内部别名才起得来的插件，核心不接回去就一直崩，先看容器的话两边互相等。
        if let Some(core) = &self.cfg.core_container {
            self.attach_core(&network, core, manifest).await?;
        }
        // 渲染和容器上的摘要对不上（核心升级换了注入的值，或旧容器没有摘要）就照安装的路径重建。全停着的
        // 插件不在这里动，留给启动流程（见 start.rs）。例外：网络刚重建时容器引用的还是已删除的旧网络，
        // 必须跟着重建，续建中的也不例外；失败照实往外报，不回退成用旧容器。
        if network_recreated || !self.recovery_pending(&record.id) {
            Box::pin(self.recreate_on_drift(
                package,
                record,
                &langs,
                false,
                gate,
                network_recreated,
            ))
            .await?;
        }
        let checked = self.check_containers(package, &langs).await?;
        self.refresh_bot_web_routes(manifest).await?;
        let devices = self.recovery_devices(manifest, record)?;
        let blocked = devices.blocked_services(package, &langs)?;
        let health = match checked.state {
            ContainersState::Stopped if blocked.is_empty() => return Ok(None),
            ContainersState::Stopped => Health::Unknown,
            ContainersState::Running(health) => health,
        };
        let problem = devices
            .problem(&langs)
            .or_else(|| checked.problem(&langs).map(|problem| problem.to_string()));
        self.ensure_relays(manifest, record).await?;
        self.check_bot(manifest).await?;
        let secrets = self.check_secrets(manifest).await?;
        let generated = secrets::generated_values(manifest, &secrets, &self.owner_languages())
            .map_err(InstallError::Storage)?;
        let proxy =
            ProxyAuth::resolve(manifest, &generated, &langs).map_err(InstallError::Storage)?;
        if let Some(secret) = &record.oidc_client_secret {
            let mut ctx = self
                .install_context(
                    manifest,
                    Some(secret),
                    generated,
                    &record.effective_permissions(manifest),
                    &langs,
                )
                .await?;
            if super::bot_web_registry::has_bot_web(manifest) {
                ctx.bots = self
                    .store
                    .bots()
                    .load(&manifest.id)?
                    .bots
                    .keys()
                    .map(|name| (name.clone(), Default::default()))
                    .collect();
                self.fill_bot_web_vars(manifest, &mut ctx)?;
            }
            self.sync_bot_web_clients(manifest, &ctx).await?;
            self.hydra(&langs)?
                .upsert_client(&client_spec(manifest, &ctx, secret))
                .await?;
        }
        let clients = if manifest
            .clients
            .as_ref()
            .is_some_and(|clients| clients.http.is_some())
        {
            let lan = self.lan_grant(package, record.lan_webui, &langs).await?;
            let mut ctx = self.rebuild_context(package, record, lan, None).await?;
            self.fill_bot_vars(package, &mut ctx, &langs).await?;
            let plan = super::render::render(package, &ctx, &langs)?;
            self.clients_upstream(manifest, &plan).await?
        } else {
            None
        };
        Ok(Some(Deployed {
            upstream: self.webui_upstream(manifest, &network).await?,
            serving: checked.serving().difference(&blocked).cloned().collect(),
            problem,
            health,
            models: self.models_upstream(manifest, &network).await?,
            mcp: self.mcp_upstream(manifest, &network, &secrets).await?,
            proxy: proxy.map(Arc::new),
            firewalled: record.firewalled,
            hosted: self.hosted_upstreams(manifest, &network).await?,
            clients,
        }))
    }

    /// 跟着重建之后核对：重建刚按 `start_period` 等过健康，这里再漂移重建也按它等。
    pub(super) async fn reconcile_running(
        &self,
        package: &PluginPackage,
        record: &InstalledRecord,
    ) -> Result<Deployed, InstallError> {
        match self.reconcile(package, record, Gate::StartPeriod).await? {
            Some(deployed) => {
                if let Some(reason) = &deployed.problem {
                    if self.recovery_pending(&record.id) {
                        self.recovery_failed(&record.id, &InstallError::Unhealthy(reason.clone()))?;
                    }
                } else {
                    self.recovery_complete(&record.id)?;
                }
                Ok(deployed)
            }
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
        let bots = self.stored_bot_names(&package.manifest.id)?;
        let mut running = 0;
        let mut healthy = true;
        let mut stopped = Vec::new();
        let mut failures = Vec::new();
        let mut missing = Vec::new();
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
                let checked = self
                    .check_webui_health(&package.manifest, service, &name, checked, langs)
                    .await;
                match checked {
                    Checked::Completed => {}
                    Checked::Healthy | Checked::Starting | Checked::Running => {
                        running += 1;
                        if checked != Checked::Healthy {
                            healthy = false;
                        }
                    }
                    Checked::Stopped => {
                        serving = false;
                        stopped.push(name);
                    }
                    Checked::Missing(reason) => {
                        serving = false;
                        missing.push(reason);
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
        Ok(classify(
            running, healthy, stopped, missing, failures, served, langs,
        ))
    }

    /// 插件的网络不在（开发栈 down、Docker 数据丢了）就按安装的规格重建：核对只核对已存在的网络，
    /// 少了它启动和核对都过不去，主人不该因此重装。网络在时不动，返回没有重建过；规格对不上由核对
    /// 照旧报冲突。重建后要把出口容器接回去（安装时是这一步建的端点），不然插件起来了也出不了网，
    /// 并重新登记地址段（网关据此认出插件的请求）。返回有没有重建过：重建后容器引用的还是旧网络，
    /// 调用方要按安装的路径重建容器（见 [`Self::recreate_on_drift`]）。
    pub(super) async fn recreate_missing_network(
        &self,
        manifest: &Manifest,
    ) -> Result<bool, InstallError> {
        let network = self.cfg.instance.network(&manifest.id.slug());
        let reach = self.network_reach(manifest);
        if self.engine.network_reach(&network).await?.is_some() {
            return Ok(false);
        }
        tracing::info!(plugin = %manifest.id, %network, "插件的网络不在，按安装的规格重建");
        if let Err(error) = self
            .engine
            .create_network(&network, &self.cfg.instance.labels(&manifest.id), reach)
            .await
        {
            let error: InstallError = error.into();
            tracing::error!(
                plugin = %manifest.id,
                operation = "reconcile",
                %network,
                error_kind = error.kind(),
                reason = %error,
                "插件网络重建失败"
            );
            return Err(error);
        }
        if reach == NetworkReach::Egress {
            self.attach_relay(&network).await?;
        }
        self.register_source(manifest, &network).await?;
        tracing::info!(
            plugin = %manifest.id,
            operation = "reconcile",
            %network,
            "插件网络重建完成"
        );
        Ok(true)
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
