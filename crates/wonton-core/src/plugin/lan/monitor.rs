//! 地址变了自动跟上（ADR-0019）：检测一次，发布的地址和选出的不一样时，按安装记录重新渲染，重建开端口或插值用到
//! 地址的容器（数据不动），再核对整个插件；选不出地址时把插件标为异常，端口不改开到别的地址上。
//!
//! 重建按记录重新渲染，要的东西都得在核心里存着：生成的密钥、OIDC 客户端密钥、模型网关凭据都在；Bot 的访问令牌
//! 不在，所以 `network.lan` 不能和 `matrix.bot` 一起申请（[`super::validate`]）。

use std::collections::BTreeSet;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

use tokio::time::Instant;
use wonton_protocol::{LanChoice, LanFailure};

use super::select::{self, Bound, Host, Plan, Problem};
use super::{LanGrant, LanSpec, published};
use crate::l10n;
use crate::plugin::health_gate::Gate;
use crate::plugin::languages::Languages;
use crate::plugin::lifecycle::Undo;
use crate::plugin::manager::{Deployed, Entry, InstallError, PluginManager, PluginStatus};
use crate::plugin::manifest::PluginId;
use crate::plugin::package::PluginPackage;
use crate::plugin::render::{Deployment, InstallContext, render};
use crate::plugin::secrets;
use crate::plugin::store::InstalledRecord;

/// 多久检测一次。地址很少变，DHCP 换了地址最多这么久跟上。
pub const INTERVAL: Duration = Duration::from_secs(5 * 60);

/// 一次检测的结果。
pub struct Detected {
    /// 检测宿主的网卡失败时的原因。用 `WONTON_LAN_ADDRESSES` 或宣告地址时照样用那些地址。
    pub failure: Option<String>,
    pub host: Host,
    /// 端口开在哪、告诉手机去哪，或者选不出的原因。
    pub result: Result<Plan, Problem>,
}

impl Detected {
    fn summary(&self, langs: &Languages) -> String {
        let result = match &self.result {
            Ok(plan) if plan.has_announce() => format!(
                "{} → {}",
                super::grant::bound_text(&plan.bind, langs),
                super::grant::describe(plan, langs)
            ),
            Ok(plan) => super::grant::describe(plan, langs),
            Err(problem) => problem.message.clone(),
        };
        match &self.failure {
            Some(failure) => {
                let failed = l10n::text(
                    langs,
                    "plugin-settings-lan-detect-failed",
                    &[("failure", failure.as_str().into())],
                );
                format!("{result}（{failed}）")
            }
            None => result,
        }
    }
}

fn lan_services(spec: &LanSpec) -> BTreeSet<&str> {
    spec.ports.iter().map(|p| p.service.as_str()).collect()
}

impl PluginManager {
    /// 当场检测一次，按主人的选择算出地址。Docker 连不上、读不了主人的选择时报错。
    pub(in crate::plugin) async fn detect_lan(
        &self,
        langs: &Languages,
    ) -> Result<Detected, InstallError> {
        let instance = &self.cfg.instance;
        let network = self
            .lan
            .detector
            .detect(&self.engine, &instance.lan_probe(), instance.core_labels())
            .await?;
        let choice = self.store.lan_choice()?.unwrap_or(LanChoice::Auto);
        let fixed = self.lan.fixed.as_deref();
        let announce = self.lan.announce.as_deref();
        let network = match network {
            Ok(network) => network,
            Err(failure) => {
                let result = match (announce, fixed) {
                    (Some(announce), _) => Ok(select::announced(announce)),
                    (None, Some(fixed)) => Ok(Plan {
                        bind: fixed
                            .iter()
                            .map(|&ip| Bound {
                                ip,
                                interface: None,
                            })
                            .collect(),
                        announce: fixed.to_vec(),
                    }),
                    (None, None) => Err(Problem::new(
                        LanFailure::DetectionFailed,
                        l10n::text(
                            langs,
                            "plugin-settings-lan-detect-failed",
                            &[("failure", failure.as_str().into())],
                        ),
                    )),
                };
                return Ok(Detected {
                    failure: Some(failure),
                    host: Host::default(),
                    result,
                });
            }
        };
        let host = select::classify(&network, &self.engine.bridges().await?, langs);
        let result = select::resolve(&host, &choice, fixed, announce, langs);
        Ok(Detected {
            failure: None,
            host,
            result,
        })
    }

    /// 检测一次，按结果更新开了局域网端口的插件，结论变了才写日志。
    pub(in crate::plugin) async fn lan_pass(
        &self,
        langs: &Languages,
    ) -> Result<Detected, InstallError> {
        let _pass = self.lan.pass.lock().await;
        let detected = self.detect_lan(langs).await?;
        if self.lan.changed(detected.summary(langs)) {
            match &detected.result {
                Ok(_) => tracing::info!(addresses = %detected.summary(langs), "局域网地址"),
                Err(_) => tracing::warn!(problem = %detected.summary(langs), "选不出局域网地址"),
            }
        }
        self.follow_egress(&detected).await;
        self.finish_closing().await;
        let plan = match &detected.result {
            Ok(plan) => Ok(plan.clone()),
            Err(problem) => Err(problem.message.clone()),
        };
        if let Ok(plan) = &plan {
            let announce: BTreeSet<IpAddr> = plan.announce.iter().copied().collect();
            self.lan.addresses.send_if_modified(|current| {
                let changed = *current != announce;
                current.clone_from(&announce);
                changed
            });
        }
        for (id, spec) in self.lan_plugins() {
            self.follow_lan(&id, &spec, &plan, langs).await;
        }
        Ok(detected)
    }

    /// 关开关失败、端口还发布着的插件接着关。已经关了的不再重建，网页不用每次检测都断一下；恢复交给 [`Self::follow_lan`]。
    async fn finish_closing(&self) {
        let closing: Vec<(PluginId, LanSpec)> = {
            let plugins = self.plugins.read().expect("插件表锁中毒");
            plugins
                .iter()
                .filter(|(id, _)| self.lan.is_degraded(id))
                .filter_map(|(id, entry)| {
                    let listed = entry.listed.as_ref().filter(|l| !l.lan_webui)?;
                    let ports = listed.lan_switched.as_ref()?.ports.clone();
                    Some((
                        id.clone(),
                        LanSpec {
                            ports,
                            client: None,
                        },
                    ))
                })
                .collect()
        };
        for (id, spec) in closing {
            let closed = matches!(
                self.lan_published(&id, &spec).await,
                Ok(Some(addresses)) if addresses.is_empty()
            );
            let langs = self.owner_languages();
            if !closed && let Err(e) = self.change_lan_webui(&id, false, &langs).await {
                tracing::warn!(plugin = %id, error = %e, "接着关网页的局域网端口失败");
            }
        }
    }

    /// 开着局域网端口的插件和开着的端口。因为局域网异常的插件端口全关了也在内：要靠检测恢复。
    pub(in crate::plugin) fn lan_plugins(&self) -> Vec<(PluginId, LanSpec)> {
        let plugins = self.plugins.read().expect("插件表锁中毒");
        plugins
            .iter()
            .filter_map(|(id, entry)| {
                let listed = entry.listed.as_ref()?;
                let ports = listed.open_ports(listed.lan_webui);
                (!ports.is_empty() || self.lan.is_degraded(id)).then(|| {
                    (
                        id.clone(),
                        LanSpec {
                            ports,
                            client: None,
                        },
                    )
                })
            })
            .collect()
    }

    /// 让一个插件跟上这次检测的结果。只碰在运行的和因为局域网异常的插件：安装、卸载中的不碰，别的原因异常的
    /// （比如卸载没做完）也不碰。
    async fn follow_lan(
        &self,
        id: &PluginId,
        spec: &LanSpec,
        plan: &Result<Plan, String>,
        langs: &Languages,
    ) {
        let running = self.status_of(id) == Some(PluginStatus::Running);
        if !running && !self.lan.is_degraded(id) {
            return;
        }
        // 端口全关了（关开关失败后还异常着）：不看地址，只核对。
        let plan = match plan {
            _ if spec.ports.is_empty() => None,
            Ok(plan) => Some(plan),
            Err(reason) => {
                return self.lan_failed(
                    id,
                    l10n::text(
                        langs,
                        "plugin-settings-lan-select-failed",
                        &[("reason", reason.as_str().into())],
                    ),
                );
            }
        };
        let moved = match plan {
            None => None,
            Some(plan) => match self.lan_published(id, spec).await {
                Ok(current) => {
                    let wanted: BTreeSet<IpAddr> = plan.bind.iter().map(|bound| bound.ip).collect();
                    (current.as_ref() != Some(&wanted)).then_some(plan)
                }
                Err(e) => {
                    return self.lan_failed(
                        id,
                        l10n::text(
                            langs,
                            "plugin-settings-lan-lookup-failed",
                            &[("error", e.to_string().into())],
                        ),
                    );
                }
            },
        };
        if moved.is_none() && running {
            return;
        }
        if !self.begin_rebuild(id) {
            return;
        }
        let result = self.relocate(id, moved, langs).await;
        self.lan.end(id);
        match result {
            Ok(deployed) => {
                tracing::info!(plugin = %id, rebuilt = moved.is_some(), "插件已跟上局域网地址");
                self.lan.clear_degraded(id);
                self.lan_restored(id, deployed);
            }
            Err(e) if moved.is_some() => self.lan_failed(
                id,
                l10n::text(
                    langs,
                    "plugin-settings-lan-rebuild-failed",
                    &[("error", e.to_string().into())],
                ),
            ),
            Err(e) => self.lan_failed(
                id,
                l10n::text(
                    langs,
                    "plugin-settings-lan-check-failed",
                    &[("error", e.to_string().into())],
                ),
            ),
        }
    }

    /// 开了局域网端口的容器都发布在同一组地址上时返回这组地址；有容器不在、或者各自发布的地址不一样时为 `None`。
    async fn lan_published(
        &self,
        id: &PluginId,
        spec: &LanSpec,
    ) -> Result<Option<BTreeSet<IpAddr>>, InstallError> {
        let mut found: Option<BTreeSet<IpAddr>> = None;
        for service in lan_services(spec) {
            let name = self.cfg.instance.container(&id.slug(), service);
            let Some(host) = self
                .engine
                .inspect_container(&name)
                .await?
                .and_then(|c| c.host_config)
            else {
                return Ok(None);
            };
            let addresses = published(&host);
            if found.as_ref().is_some_and(|f| *f != addresses) {
                return Ok(None);
            }
            found = Some(addresses);
        }
        Ok(found)
    }

    /// 插件还在运行或异常（没在安装、卸载）时记为正在重建，返回记上了没有。和卸载一样在插件表的锁里判断。
    pub(in crate::plugin) fn begin_rebuild(&self, id: &PluginId) -> bool {
        let plugins = self.plugins.write().expect("插件表锁中毒");
        let idle = matches!(
            plugins.get(id).map(|e| &e.status),
            Some(PluginStatus::Running | PluginStatus::Stopped | PluginStatus::Degraded { .. })
        );
        idle && self.lan.begin(id)
    }

    /// `plan` 为空时地址没变，只核对插件（异常的插件恢复过来）。
    async fn relocate(
        &self,
        id: &PluginId,
        plan: Option<&Plan>,
        langs: &Languages,
    ) -> Result<Deployed, InstallError> {
        let record = self
            .store
            .load(id)?
            .ok_or_else(|| InstallError::not_installed(langs, id))?;
        let mut package = PluginPackage::with_files(&record.source, record.files.clone(), langs)?;
        package.origin = record.origin.clone();
        if let Some(plan) = plan {
            self.rebuild_lan(&package, &record, plan).await?;
        }
        self.reconcile_running(&package, &record).await
    }

    /// 照安装时一样渲染，重建局域网端口、插值用到地址、以及共用它们网络命名空间的服务。
    pub(super) async fn rebuild_lan(
        &self,
        package: &PluginPackage,
        record: &InstalledRecord,
        plan: &Plan,
    ) -> Result<(), InstallError> {
        // 要迁就先预留+按新网段渲染：渲染失败放掉预留，旧网络和旧容器一个不动，插件照原样跑。
        let (pending, reservation) = match self.prepare_migration(&package.manifest).await? {
            Some((pending, reservation)) => (Some(pending), Some(reservation)),
            None => (None, None),
        };
        let bind: Vec<IpAddr> = plan.bind.iter().map(|bound| bound.ip).collect();
        let grant = self.lan_grant_on(package, record.lan_webui, bind, plan.announce.clone());
        let mut ctx = self
            .rebuild_context(package, record, Some(grant), pending.as_ref())
            .await?;
        let langs = self.owner_languages();
        // Bot 的 Matrix 变量和总线地址也要填：Bot 服务的容器引用了它们，令牌从旧容器读回。
        self.fill_bot_vars(package, &mut ctx, &langs).await?;
        let mut render_plan = render(package, &ctx, &langs)?;
        match (&pending, reservation) {
            (Some(pending), Some(reservation)) => {
                self.apply_migration(
                    &package.manifest,
                    pending,
                    reservation,
                    ctx.egress.is_some(),
                    &langs,
                )
                .await?;
            }
            (None, None) => {
                let mut services: BTreeSet<String> = package
                    .manifest
                    .lan
                    .iter()
                    .flat_map(|lan| &lan.ports)
                    .map(|port| port.service.clone())
                    .collect();
                services.extend(
                    render_plan
                        .containers
                        .iter()
                        .filter(|c| c.variables.contains(super::ADDRESSES_VAR))
                        .map(|c| c.service.clone()),
                );
                render_plan.retain_services(&services);
            }
            _ => unreachable!("迁移标记和预留成对出现"),
        }
        // 局域网重建是主人正在等着的一次性操作，和安装、启动同一条规则。
        self.replace_containers(&ctx, render_plan, Gate::StartPeriod)
            .await
    }

    /// 重建已装插件的容器时的上下文：和安装时一样，凭据用存下的。Bot 的 Matrix 变量另外填（见 [`crate::plugin::egress`]）。
    pub(in crate::plugin) async fn rebuild_context(
        &self,
        package: &PluginPackage,
        record: &InstalledRecord,
        lan: Option<LanGrant>,
        migration: Option<&super::super::network_migration::PendingMigration>,
    ) -> Result<InstallContext, InstallError> {
        let manifest = &package.manifest;
        let secrets = self.check_secrets(manifest).await?;
        let langs = self.owner_languages();
        let generated =
            secrets::generated_values(manifest, &secrets, &langs).map_err(InstallError::Storage)?;
        let secret = record.oidc_client_secret.as_deref();
        let mut ctx = self
            .install_context(
                manifest,
                secret,
                generated,
                &record.effective_permissions(manifest),
                &langs,
            )
            .await?;
        ctx.limits = record.limits;
        ctx.settings = record.settings.clone();
        ctx.gpu = self
            .gpu_grant(manifest, &record.effective_permissions(manifest), &langs)
            .await?;
        ctx.models = self.stored_model_vars(package)?;
        if let Some(network) = lan.as_ref().and_then(|grant| grant.network.as_ref())
            && self.engine.network_reach(network).await?.is_none()
        {
            let labels = self.cfg.instance.labels(&manifest.id);
            match migration.and_then(|m| m.lan) {
                Some(subnet) => {
                    self.engine
                        .create_lan_network_at(network, &labels, subnet)
                        .await?
                }
                None => self.engine.create_lan_network(network, &labels).await?,
            }
        }
        ctx.lan = lan;
        let network = self.cfg.instance.network(&ctx.slug);
        match migration.and_then(|m| m.main) {
            // 主网络要迁：变量按预留的新网段算（渲染成功才动旧网络）。
            Some(_) => {
                ctx.extra_hosts = migration.unwrap().extra_hosts.clone();
                ctx.core_address = migration.unwrap().core_address.clone();
            }
            None => {
                ctx.extra_hosts = self.hosts_via_gateway(manifest, &network).await?;
                ctx.core_address = self.core_address_var(manifest, &network).await?;
            }
        }
        Ok(ctx)
    }

    /// 先把镜像拉好，再换掉计划里的容器：拉不到时旧容器还在跑。等健康的时限按 `gate` 取。
    pub(in crate::plugin) async fn replace_containers(
        &self,
        ctx: &InstallContext,
        mut plan: Deployment,
        gate: Gate,
    ) -> Result<(), InstallError> {
        let langs = self.owner_languages();
        self.prepare_images(&mut plan, None, &langs).await?;
        // 计划按依赖拓扑排序，正序删除让共享网络命名空间的容器先于目标删；逆序会把目标先删，等价于按容器 ID 绑定的依赖先拆引用者。
        for container in plan.containers.iter() {
            self.engine.remove_container(&container.name).await?;
        }
        self.rewrite_image_healthchecks(ctx, &mut plan).await?;
        let deadline = gate.deadline(&plan, ctx.install_limit, Instant::now());
        Box::pin(self.start_containers(None, &plan, None, &mut Undo::default(), deadline, &langs))
            .await?;
        Ok(())
    }

    /// 标为异常，以后每次检测都重新处理它。只改在运行的和已经因为局域网异常的插件。
    pub(super) fn lan_failed(&self, id: &PluginId, reason: String) {
        tracing::warn!(plugin = %id, %reason, "插件的局域网端口异常");
        let mut plugins = self.plugins.write().expect("插件表锁中毒");
        let Some(entry) = plugins.get_mut(id) else {
            return;
        };
        let ours = entry.status == PluginStatus::Running || self.lan.is_degraded(id);
        let Some(listed) = entry.listed.clone().filter(|_| ours) else {
            return;
        };
        *entry = Entry::new(listed, PluginStatus::Degraded { reason }, None);
        self.lan.mark_degraded(id);
        drop(plugins);
        self.changed();
    }

    fn lan_restored(&self, id: &PluginId, deployed: Deployed) {
        let mut plugins = self.plugins.write().expect("插件表锁中毒");
        if let Some(entry) = plugins.get_mut(id)
            && let Some(listed) = entry.listed.clone()
        {
            *entry = Entry::running(listed, deployed);
        }
        drop(plugins);
        self.changed();
    }

    /// 选出的局域网地址：隧道把它们宣告给手机，跟着检测结果变（ADR-0032）。
    pub fn lan_addresses(&self) -> tokio::sync::watch::Receiver<BTreeSet<IpAddr>> {
        self.lan.addresses.subscribe()
    }

    /// 核心启动后检测一次，之后每 [`INTERVAL`] 一次。
    pub fn spawn_lan_monitor(self: &Arc<Self>) {
        let this = Arc::downgrade(self);
        tokio::spawn(async move {
            let mut ticks = tokio::time::interval(INTERVAL);
            loop {
                ticks.tick().await;
                let Some(manager) = this.upgrade() else {
                    return;
                };
                if let Err(e) = manager.lan_pass(&manager.owner_languages()).await {
                    tracing::warn!(error = %e, "检测局域网地址失败");
                }
            }
        });
    }
}
