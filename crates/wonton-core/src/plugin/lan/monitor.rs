//! 地址变了自动跟上（ADR-0019）：检测一次，开了局域网端口的容器发布的地址和选出的不一样，就照安装时一样重新
//! 渲染、只重建这些容器（数据不动），再核对整个插件；选不出地址时把插件标为异常，端口不改开到别的地址上。
//!
//! 重建按记录重新渲染，要的东西都得在核心里存着：生成的密钥、OIDC 客户端密钥、模型网关凭据都在；Bot 的访问令牌
//! 不在，所以 `network.lan` 不能和 `matrix.bot` 一起申请（[`super::validate`]）。

use std::collections::BTreeSet;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

use tokio::time::Instant;
use wonton_protocol::{LanChoice, LanFailure};

use super::select::{self, Bound, Host, Problem};
use super::{LanSpec, published};
use crate::plugin::lifecycle::Undo;
use crate::plugin::manager::{Deployed, Entry, InstallError, PluginManager, PluginStatus};
use crate::plugin::manifest::PluginId;
use crate::plugin::package::PluginPackage;
use crate::plugin::render::render;
use crate::plugin::store::InstalledRecord;

/// 多久检测一次。地址很少变，DHCP 换了地址最多这么久跟上。
pub const INTERVAL: Duration = Duration::from_secs(5 * 60);

/// 一次检测的结果。
pub struct Detected {
    /// 检测宿主的网卡失败时的原因。用 `WONTON_LAN_ADDRESSES` 时照样用那些地址。
    pub failure: Option<String>,
    pub host: Host,
    /// 端口开在哪些地址上，或者选不出的原因。
    pub result: Result<Vec<Bound>, Problem>,
}

impl Detected {
    fn summary(&self) -> String {
        let result = match &self.result {
            Ok(bound) => super::grant::describe(bound),
            Err(problem) => problem.message.clone(),
        };
        match &self.failure {
            Some(failure) => format!("{result}（检测宿主的网卡失败：{failure}）"),
            None => result,
        }
    }
}

fn lan_services(spec: &LanSpec) -> BTreeSet<&str> {
    spec.ports.iter().map(|p| p.service.as_str()).collect()
}

impl PluginManager {
    /// 当场检测一次，按主人的选择算出地址。Docker 连不上、读不了主人的选择时报错。
    pub(in crate::plugin) async fn detect_lan(&self) -> Result<Detected, InstallError> {
        let instance = &self.cfg.instance;
        let network = self
            .lan
            .detector
            .detect(&self.engine, &instance.lan_probe(), instance.core_labels())
            .await?;
        let choice = self.store.lan_choice()?.unwrap_or(LanChoice::Auto);
        let fixed = self.lan.fixed.as_deref();
        let network = match network {
            Ok(network) => network,
            Err(failure) => {
                let result = match fixed {
                    Some(fixed) => Ok(fixed
                        .iter()
                        .map(|&ip| Bound {
                            ip,
                            interface: None,
                        })
                        .collect()),
                    None => Err(Problem::new(
                        LanFailure::DetectionFailed,
                        format!("检测宿主的网卡失败：{failure}"),
                    )),
                };
                return Ok(Detected {
                    failure: Some(failure),
                    host: Host::default(),
                    result,
                });
            }
        };
        let host = select::classify(&network, &self.engine.bridges().await?);
        let result = select::resolve(&host, &choice, fixed);
        Ok(Detected {
            failure: None,
            host,
            result,
        })
    }

    /// 检测一次，按结果更新开了局域网端口的插件，结论变了才写日志。
    pub(in crate::plugin) async fn lan_pass(&self) -> Result<Detected, InstallError> {
        let _pass = self.lan.pass.lock().await;
        let detected = self.detect_lan().await?;
        if self.lan.changed(detected.summary()) {
            match &detected.result {
                Ok(_) => tracing::info!(addresses = %detected.summary(), "局域网地址"),
                Err(_) => tracing::warn!(problem = %detected.summary(), "选不出局域网地址"),
            }
        }
        let addresses = detected
            .result
            .as_ref()
            .map(|bound| bound.iter().map(|b| b.ip).collect::<BTreeSet<IpAddr>>())
            .map_err(|p| p.message.clone());
        for (id, spec) in self.lan_plugins() {
            self.follow_lan(&id, &spec, &addresses).await;
        }
        Ok(detected)
    }

    /// 开着局域网端口的插件和开着的端口。
    pub(in crate::plugin) fn lan_plugins(&self) -> Vec<(PluginId, LanSpec)> {
        let plugins = self.plugins.read().expect("插件表锁中毒");
        plugins
            .iter()
            .filter_map(|(id, entry)| {
                let listed = entry.listed.as_ref()?;
                let ports = listed.open_ports(listed.lan_webui);
                (!ports.is_empty()).then(|| (id.clone(), LanSpec { ports }))
            })
            .collect()
    }

    /// 让一个插件跟上这次检测的结果。只碰在运行的和因为局域网异常的插件：安装、卸载中的不碰，别的原因异常的
    /// （比如卸载没做完）也不碰。
    async fn follow_lan(
        &self,
        id: &PluginId,
        spec: &LanSpec,
        addresses: &Result<BTreeSet<IpAddr>, String>,
    ) {
        let running = self.status_of(id) == Some(PluginStatus::Running);
        if !running && !self.lan.is_degraded(id) {
            return;
        }
        let addresses = match addresses {
            Ok(addresses) => addresses,
            Err(reason) => {
                return self.lan_failed(id, format!("选不出局域网地址：{reason}"));
            }
        };
        let current = match self.lan_published(id, spec).await {
            Ok(current) => current,
            Err(e) => return self.lan_failed(id, format!("查不到局域网端口发布在哪里：{e}")),
        };
        let same = current.as_ref() == Some(addresses);
        if same && running {
            return;
        }
        if !self.begin_rebuild(id) {
            return;
        }
        let result = self.relocate(id, spec, (!same).then_some(addresses)).await;
        self.lan.end(id);
        match result {
            Ok(deployed) => {
                tracing::info!(plugin = %id, rebuilt = !same, "插件已跟上局域网地址");
                self.lan.clear_degraded(id);
                self.lan_restored(id, deployed);
            }
            Err(e) => self.lan_failed(id, format!("按新的局域网地址重建失败：{e}")),
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
    pub(super) fn begin_rebuild(&self, id: &PluginId) -> bool {
        let plugins = self.plugins.write().expect("插件表锁中毒");
        let idle = matches!(
            plugins.get(id).map(|e| &e.status),
            Some(PluginStatus::Running | PluginStatus::Degraded { .. })
        );
        idle && self.lan.begin(id)
    }

    /// `addresses` 为空时地址没变，只核对插件（异常的插件恢复过来）。
    async fn relocate(
        &self,
        id: &PluginId,
        spec: &LanSpec,
        addresses: Option<&BTreeSet<IpAddr>>,
    ) -> Result<Deployed, InstallError> {
        let record = self
            .store
            .load(id)?
            .ok_or_else(|| InstallError::NotInstalled(id.to_string()))?;
        let mut package = PluginPackage::with_files(&record.source, record.files.clone())?;
        package.origin = record.origin.clone();
        if let Some(addresses) = addresses {
            let addresses: Vec<IpAddr> = addresses.iter().copied().collect();
            self.rebuild_lan(&package, &record, &addresses, &lan_services(spec))
                .await?;
        }
        self.reconcile(&package, &record).await
    }

    /// 照安装时一样渲染，只删掉、重建 `services` 的容器。
    pub(super) async fn rebuild_lan(
        &self,
        package: &PluginPackage,
        record: &InstalledRecord,
        addresses: &[IpAddr],
        services: &BTreeSet<&str>,
    ) -> Result<(), InstallError> {
        let manifest = &package.manifest;
        let generated = self.check_secrets(manifest).await?;
        let secret = record.oidc_client_secret.as_deref();
        let mut ctx = self.install_context(manifest, secret, generated).await?;
        ctx.gpu = self.gpu_grant(manifest).await?;
        ctx.models = self.stored_model_vars(package)?;
        let grant = self.lan_grant_on(package, record.lan_webui, addresses.to_vec());
        if let Some(network) = &grant.network
            && self.engine.network_reach(network).await?.is_none()
        {
            let labels = self.cfg.instance.labels(&manifest.id);
            self.engine.create_lan_network(network, &labels).await?;
        }
        ctx.lan = Some(grant);
        let network = self.cfg.instance.network(&ctx.slug);
        ctx.extra_hosts = self.hosts_via_gateway(manifest, &network).await?;
        ctx.core_address = self.core_address_var(manifest, &network).await?;
        let mut plan = render(package, &ctx)?;
        plan.containers
            .retain(|c| services.contains(c.service.as_str()));
        for container in &plan.containers {
            self.engine
                .ensure_image(&container.image, container.pull_policy)
                .await?;
            self.engine.remove_container(&container.name).await?;
        }
        self.rewrite_image_healthchecks(&ctx, &mut plan).await?;
        let deadline = Instant::now() + self.cfg.install_timeout;
        self.start_containers(&plan, &mut Undo::default(), deadline)
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

    /// 核心启动后检测一次，之后每 [`INTERVAL`] 一次。没装开局域网端口的插件时不检测：检测要起一个容器。
    pub fn spawn_lan_monitor(self: &Arc<Self>) {
        let this = Arc::downgrade(self);
        tokio::spawn(async move {
            let mut ticks = tokio::time::interval(INTERVAL);
            loop {
                ticks.tick().await;
                let Some(manager) = this.upgrade() else {
                    return;
                };
                if manager.lan_plugins().is_empty() {
                    continue;
                }
                if let Err(e) = manager.lan_pass().await {
                    tracing::warn!(error = %e, "检测局域网地址失败");
                }
            }
        });
    }
}
