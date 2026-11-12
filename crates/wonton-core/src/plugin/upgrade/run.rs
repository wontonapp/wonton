//! 执行升级：保留数据、插件密钥、Bot 账号和设备、模型凭据、OIDC 客户端，只换容器。
//!
//! 先把能失败的都做在旧容器还在跑的时候（渲染、拉镜像、建目录和卷），这些失败了旧版本不受影响。换容器之后失败就退回：用旧记录和旧容器当时的镜像重建旧版本。Bot 的令牌从旧容器读回（见 [`crate::plugin::egress`]），只留在内存里和容器的环境
//! 变量里，这里绝不签新设备：换设备要重新建加密身份。安装记录最后才写，核心中途退出时记录还是旧版本的。

use std::collections::HashMap;
use std::sync::Arc;

use tokio::time::Instant;
use wonton_protocol::InstallStage;

use super::super::bots::is_bot;
use super::super::compose::PullPolicy;
use super::super::gpu::GpuGrant;
use super::super::health_gate::Gate;
use super::super::lan::LanGrant;
use super::super::lan::webui::switched;
use super::super::languages::Languages;
use super::super::lifecycle::{Undo, VolumeAction, client_spec, volume_action};
use super::super::listed::Listed;
use super::super::manager::{
    Deployed, Entry, Health, InstallError, PluginManager, PluginStatus, busy, check_permissions,
    now_secs,
};
use super::super::manifest::{Manifest, Permission, PluginId, PluginKind};
use super::super::package::PluginPackage;
use super::super::proxy_auth::ProxyAuth;
use wonton_protocol::BotKeeper;

use super::super::render::{Deployment, InstallContext, dry_run, render};
use super::super::secrets::{self, Secrets};
use super::super::store::{EnabledPermission, InstalledRecord};
use super::super::upstreams::uses_sso;
use super::{installed_package, newer};
use crate::l10n;
use crate::secret::random_token;

fn old_limits(entry: &Entry) -> wonton_protocol::PluginLimits {
    entry
        .listed
        .as_ref()
        .map(|listed| listed.limits)
        .unwrap_or_default()
}

/// 退回旧版本要用的：Bot 的令牌在 `ctx` 里。
struct Old {
    record: InstalledRecord,
    package: PluginPackage,
    ctx: InstallContext,
    /// 服务 → 旧容器当时的镜像：标签可能已经被新版本的拉取移动了。
    images: HashMap<String, String>,
}

/// 做到哪了，失败时据此收拾。
#[derive(Default)]
struct Progress {
    undo: Undo,
    /// 旧容器动过了：失败时要重建旧版本。
    touched: bool,
    network_rebuilt: bool,
    /// 这次多签发的模型网关凭据。
    added_models: Vec<String>,
}

/// 新版本部署要用的，都在旧容器还在跑的时候备好。
struct Prepared {
    ctx: InstallContext,
    secrets: Secrets,
    proxy: Option<ProxyAuth>,
    oidc_secret: Option<String>,
    pulled: Deployment,
}

enum Outcome {
    Upgraded(Deployed),
    /// 旧容器没动过：状态放回去。
    Untouched(InstallError),
    /// 换过容器，退回了旧版本。
    RolledBack(Arc<Listed>, Deployed, InstallError),
    RollbackFailed(Arc<Listed>, InstallError),
}

/// 旧容器之前的样子，旧容器没动过时放回去。
struct Before {
    status: PluginStatus,
    health: Option<Health>,
}

fn pin_images(plan: &mut Deployment, images: &HashMap<String, String>) {
    for container in &mut plan.containers {
        if let Some(image) = images.get(&container.service) {
            container.image.clone_from(image);
            container.body.image = Some(image.clone());
            container.pull_policy = PullPolicy::Missing;
        }
    }
}

impl PluginManager {
    /// 检查并升级。`granted` 必须含新版本的全部必需权限：这就是主人的确认；`requested` 是主人勾选的可选项。
    pub(super) async fn upgrade_declared(
        &self,
        package: PluginPackage,
        granted: &[Permission],
        requested: &[EnabledPermission],
        lan_webui: bool,
        settings: super::super::settings::Values,
    ) -> Result<(), InstallError> {
        let langs = self.owner_languages();
        super::super::install_entry::check_runtime(&package.manifest, &langs)?;
        self.check_not_upgrading(&langs)?;
        if !super::super::install_hosted::is_containerless(&package.manifest) {
            dry_run(&package, &langs)?;
        }
        self.check_icon(&package, &langs).await?;
        let manifest = &package.manifest;
        let id = &manifest.id;
        // 只有新版本还声明成可选的才留下；以前开着的沿用原来的时间和人（ADR-0057）。
        let record = self
            .store
            .load(id)?
            .ok_or_else(|| InstallError::NotInstalled(id.to_string()))?;
        let enabled: Vec<EnabledPermission> = requested
            .iter()
            .filter(|item| manifest.optional_permissions.contains_key(&item.permission))
            .map(|item| {
                record
                    .enabled_permissions
                    .iter()
                    .find(|old| old.permission == item.permission)
                    .cloned()
                    .unwrap_or_else(|| item.clone())
            })
            .collect();
        check_permissions(manifest, granted, &enabled, &langs)?;
        super::super::capabilities::unsupported_model_dependency(manifest, &langs)?;
        if super::super::install_hosted::is_containerless(manifest) {
            return self.upgrade_containerless(package, granted, &langs).await;
        }
        self.check_hosted_provider(manifest, &langs)?;
        self.check_requirements(manifest, &langs).await?;
        self.check_egress(manifest, &langs).await?;
        let gpu = self
            .gpu_grant(
                manifest,
                &manifest.effective_permissions(&super::super::store::permission_set(&enabled)),
                &langs,
            )
            .await?;
        let lan_webui = lan_webui && switched(&package).is_some();
        let lan = self.lan_grant(&package, lan_webui, &langs).await?;
        let before = self.begin_upgrade(id)?;
        tracing::info!(plugin = %id, version = %manifest.version, "开始升级插件");
        // 放堆上跑：从建容器到对账的一整串都在这条链里，调试构建的帧很大，嵌套两层就够顶满测试线程的 2 MiB 栈。
        let outcome = Box::pin(self.upgrade_begun(
            &package,
            granted,
            &enabled,
            (gpu, lan),
            settings.clone(),
            &langs,
        ))
        .await;
        self.lan.end(id);
        let result = self.finish_upgrade(&package, lan_webui, before, outcome);
        if result.is_ok()
            && super::super::egress::provider::provides_proxy(manifest)
            && self.egress_config()?.source == wonton_protocol::EgressProxySource::Plugin
        {
            self.follow_core_proxy();
            self.reconcile_relay().await?;
        }
        self.changed();
        result
    }

    /// 和卸载一样在插件表的锁里判断：在安装、卸载、升级或者按新地址重建的不碰。
    fn begin_upgrade(&self, id: &PluginId) -> Result<Before, InstallError> {
        let langs = self.owner_languages();
        let mut plugins = self.plugins.write().expect("插件表锁中毒");
        let entry = plugins
            .get_mut(id)
            .ok_or_else(|| InstallError::not_installed(&langs, id))?;
        if let Some(e) = busy(id, &entry.status, &langs) {
            return Err(e);
        }
        if !self.lan.begin(id) {
            return Err(InstallError::Conflict(l10n::text(
                &langs,
                "plugin-install-conflict-lan-rebuilding",
                &[("id", id.to_string().into())],
            )));
        }
        let before = Before {
            status: entry.status.clone(),
            health: entry.health,
        };
        entry.status = PluginStatus::Upgrading;
        entry.install_progress = None;
        drop(plugins);
        self.changed();
        Ok(before)
    }

    fn finish_upgrade(
        &self,
        package: &PluginPackage,
        lan_webui: bool,
        before: Before,
        outcome: Outcome,
    ) -> Result<(), InstallError> {
        let id = &package.manifest.id;
        let mut plugins = self.plugins.write().expect("插件表锁中毒");
        let Some(entry) = plugins.get_mut(id) else {
            let langs = self.owner_languages();
            return Err(InstallError::not_installed(&langs, id));
        };
        entry.install_progress = None;
        match outcome {
            Outcome::Upgraded(deployed) => {
                let installed_at = entry
                    .listed
                    .as_ref()
                    .map_or_else(now_secs, |l| l.installed_at);
                let listed = Listed::new(
                    package,
                    installed_at,
                    lan_webui,
                    deployed.firewalled,
                    old_limits(entry),
                );
                *entry = Entry::running(listed, deployed);
                self.lan.clear_degraded(id);
                self.upgrades.forget(id);
                tracing::info!(plugin = %id, version = %package.manifest.version, "插件升级完成");
                Ok(())
            }
            Outcome::Untouched(e) => {
                entry.status = before.status;
                entry.health = before.health;
                tracing::warn!(plugin = %id, error = %e, "插件升级失败，旧版本没动");
                Err(e)
            }
            Outcome::RolledBack(listed, deployed, e) => {
                *entry = Entry::running(listed, deployed);
                tracing::warn!(plugin = %id, error = %e, "插件升级失败，已退回旧版本");
                Err(e)
            }
            Outcome::RollbackFailed(listed, e) => {
                let reason = e.to_string();
                *entry = Entry::new(listed, PluginStatus::Degraded { reason }, None);
                tracing::error!(plugin = %id, error = %e, "插件升级失败，退回旧版本也失败");
                Err(e)
            }
        }
    }

    async fn upgrade_begun(
        &self,
        package: &PluginPackage,
        granted: &[Permission],
        enabled: &[EnabledPermission],
        grants: (Option<GpuGrant>, Option<LanGrant>),
        settings: super::super::settings::Values,
        langs: &Languages,
    ) -> Outcome {
        let old = match self.read_old(&package.manifest.id).await {
            Ok(old) => old,
            Err(e) => return Outcome::Untouched(e),
        };
        let mut progress = Progress::default();
        let error = match self
            .replace_version(
                &old,
                package,
                granted,
                enabled,
                grants,
                settings,
                &mut progress,
            )
            .await
        {
            Ok(deployed) => {
                self.drop_unused(&old, package, langs).await;
                return Outcome::Upgraded(deployed);
            }
            Err(e) => e,
        };
        let version = &old.record.version;
        let new = &package.manifest.version;
        if !progress.touched {
            return Outcome::Untouched(self.undo_prepared(&old, progress, error).await);
        }
        let listed = Listed::of_record(&old.package, &old.record);
        match self.back_to_old(&old, package, progress).await {
            Ok(deployed) => {
                let note = l10n::text(
                    langs,
                    "plugin-settings-upgrade-rolled-back",
                    &[
                        ("new", new.as_str().into()),
                        ("version", version.as_str().into()),
                    ],
                );
                Outcome::RolledBack(listed, deployed, error.with_note(langs, &note))
            }
            Err(back) => {
                let note = l10n::text(
                    langs,
                    "plugin-settings-upgrade-rollback-failed",
                    &[
                        ("new", new.as_str().into()),
                        ("version", version.as_str().into()),
                        ("error", back.to_string().into()),
                    ],
                );
                Outcome::RollbackFailed(listed, error.with_note(langs, &note))
            }
        }
    }

    /// 读回旧版本的上下文和令牌：读不到就报错，容器还没动。
    async fn read_old(&self, id: &PluginId) -> Result<Old, InstallError> {
        let langs = self.owner_languages();
        let record = self
            .store
            .load(id)?
            .ok_or_else(|| InstallError::not_installed(&langs, id))?;
        let package = installed_package(&record, &langs)?;
        let lan = self.lan_grant(&package, record.lan_webui, &langs).await?;
        let mut ctx = self.rebuild_context(&package, &record, lan, None).await?;
        // Bot 的 Matrix 变量和总线地址也要填：Bot 服务的容器引用了它们，令牌从旧容器读回。
        self.fill_bot_vars(&package, &mut ctx, &langs).await?;
        let plan = render(&package, &ctx, &langs)?;
        let mut images = HashMap::new();
        for container in &plan.containers {
            let info = self.engine.inspect_container(&container.name).await?;
            if let Some(image) = info.and_then(|info| info.image) {
                images.insert(container.service.clone(), image);
            }
        }
        Ok(Old {
            record,
            package,
            ctx,
            images,
        })
    }

    #[allow(clippy::too_many_arguments)] // 装插件要的几样各管各的来源，打包反而看不清哪来的。
    async fn replace_version(
        &self,
        old: &Old,
        package: &PluginPackage,
        granted: &[Permission],
        enabled: &[EnabledPermission],
        grants: (Option<GpuGrant>, Option<LanGrant>),
        settings: super::super::settings::Values,
        progress: &mut Progress,
    ) -> Result<Deployed, InstallError> {
        let manifest = &package.manifest;
        let prepared = self
            .prepare_new(old, package, enabled, grants, settings, progress)
            .await?;
        let langs = self.owner_languages();
        let Prepared {
            mut ctx,
            mut secrets,
            proxy,
            oidc_secret,
            pulled,
        } = prepared;
        let network = self.cfg.instance.network(&ctx.slug);
        self.swap_network(old, manifest, &network, progress).await?;
        ctx.core_address = self.core_address_var(manifest, &network).await?;
        ctx.extra_hosts = self.hosts_via_gateway(manifest, &network).await?;
        let mut plan = render(package, &ctx, &langs)?;
        super::super::registry_mirror::use_pulled_images(&mut plan, pulled);
        if let Some(secret) = &oidc_secret {
            let spec = client_spec(manifest, &ctx, secret);
            self.hydra(&langs)?.upsert_client(&spec).await?;
        }
        self.rewrite_image_healthchecks(&ctx, &mut plan).await?;
        let deadline =
            super::super::health_gate::deadline(&plan, self.cfg.install_timeout, Instant::now());
        self.stage(&manifest.id, InstallStage::Starting);
        let health = self
            .start_containers(
                Some(manifest),
                &plan,
                Some(&manifest.id),
                &mut progress.undo,
                deadline,
                &langs,
            )
            .await?;
        let (upstream, models, mcp) = self
            .deploy_upstreams(manifest, &network, &mut secrets, deadline, &langs)
            .await?;
        let firewalled = self
            .host_webui_firewalled(manifest, &plan.relays, &network)
            .await;
        if let Some((_, _, bot)) = &progress.undo.bot {
            progress.undo.handed_over = self.hand_over_home_room(&self.cfg.owner, bot).await?;
        }
        self.store.save(&InstalledRecord {
            id: manifest.id.clone(),
            version: manifest.version.clone(),
            source: package.source.clone(),
            files: package.files().clone(),
            granted_permissions: granted.to_vec(),
            enabled_permissions: enabled.to_vec(),
            limits: old.record.limits,
            lan_webui: ctx.lan.as_ref().is_some_and(|lan| lan.webui),
            relays: plan.relays.clone(),
            firewalled,
            proxied: ctx.egress.is_some(),
            oidc_client_secret: oidc_secret,
            installed_at: old.record.installed_at,
            entry_label: old.record.entry_label.clone(),
            origin: package.origin.clone(),
            // 归属跟着插件走：Agent 更新自己写的插件后还是它写的；App 里经文件或来源升级的清掉（各路径自己做）。
            written_by: old.record.written_by.clone(),
            ask_every_update: old.record.ask_every_update,
            family: old.record.family,
            settings: ctx.settings.clone().into_iter().collect(),
            settings_changed_ms: old.record.settings_changed_ms,
            static_files: Default::default(),
            hosted: Vec::new(),
        })?;
        Ok(Deployed {
            upstream,
            serving: package
                .resident_services()
                .into_iter()
                .map(str::to_owned)
                .collect(),
            problem: None,
            health,
            models,
            proxy: proxy.map(Arc::new),
            mcp,
            firewalled,
            hosted: self.hosted_upstreams(manifest, &network).await?,
        })
    }

    /// 旧容器还在跑时能做的都做了：渲染、凭据、拉镜像、目录和卷。这一步失败时旧版本不受影响。
    async fn prepare_new(
        &self,
        old: &Old,
        package: &PluginPackage,
        enabled: &[EnabledPermission],
        (gpu, lan): (Option<GpuGrant>, Option<LanGrant>),
        settings: super::super::settings::Values,
        progress: &mut Progress,
    ) -> Result<Prepared, InstallError> {
        let manifest = &package.manifest;
        self.stage(&manifest.id, InstallStage::Preparing);
        let oidc_secret = uses_sso(manifest)
            .then(|| old.record.oidc_client_secret.clone())
            .map(|kept| kept.unwrap_or_else(random_token));
        let langs = self.owner_languages();
        self.prepare_dependencies(manifest, &langs).await?;
        self.revoke_removed_provider_keys(manifest, &langs).await?;
        let secrets = self.prepare_secrets(manifest, &mut progress.undo)?;
        let generated =
            secrets::generated_values(manifest, &secrets, &langs).map_err(InstallError::Storage)?;
        let proxy =
            ProxyAuth::resolve(manifest, &generated, &langs).map_err(InstallError::Storage)?;
        let mut ctx = self
            .install_context(
                manifest,
                oidc_secret.as_deref(),
                generated,
                &manifest.effective_permissions(&super::super::store::permission_set(enabled)),
                &langs,
            )
            .await?;
        ctx.limits = old.record.limits;
        ctx.gpu = gpu;
        ctx.lan = lan;
        ctx.settings = settings;
        // 新版本才接入 Matrix 的插件：先代 Bot 登录新设备，模型凭据签发后再把变量填进上下文。
        let mut new_bot = None;
        if is_bot(manifest) && !is_bot(&old.package.manifest) {
            let keeper = BotKeeper::User {
                username: self.cfg.owner.clone(),
            };
            let bot = self
                .provision_bot(
                    &manifest.id,
                    manifest,
                    super::super::bot_services::DEFAULT_BOT,
                    keeper,
                    package,
                )
                .await?;
            progress.undo.bot = Some((
                manifest.id.clone(),
                super::super::bot_services::DEFAULT_BOT.to_owned(),
                bot.clone(),
            ));
            new_bot = Some(bot);
        }
        ctx.models = self
            .upgrade_model_vars(package, &mut progress.added_models, &langs)
            .await?;
        if let Some(bot) = new_bot {
            self.fill_default_bot_vars(manifest, &bot, &mut ctx);
        } else {
            self.fill_bot_vars(package, &mut ctx, &langs).await?;
        }
        let mut plan = render(package, &ctx, &langs)?;
        self.prepare_images(&mut plan, Some(&manifest.id), &langs)
            .await?;
        self.stage(&manifest.id, InstallStage::Preparing);
        self.prepare_dirs(&ctx, &plan.dirs, &mut progress.undo, &langs)?;
        for volume in &plan.volumes {
            let labels = self.engine.volume_labels(volume).await?;
            if volume_action(volume, labels.as_ref(), &manifest.id, &langs)? == VolumeAction::Create
            {
                self.engine.create_volume(volume, &plan.labels).await?;
                progress.undo.volumes.push(volume.clone());
            }
        }
        if let Some(network) = ctx.lan.as_ref().and_then(|l| l.network.clone())
            && self.engine.network_reach(&network).await?.is_none()
        {
            self.engine
                .create_lan_network(&network, &plan.labels)
                .await?;
            progress.undo.lan_network = Some(network);
        }
        Ok(Prepared {
            ctx,
            secrets,
            proxy,
            oidc_secret,
            pulled: plan,
        })
    }

    /// 删掉这个插件所有的容器，包括新版本不要了的服务的。
    async fn remove_own_containers(&self, id: &PluginId) -> Result<(), InstallError> {
        for container in &self.own_resources(id).await?.containers {
            self.engine.remove_container(container).await?;
        }
        Ok(())
    }

    /// 删掉旧容器，让插件网络变成新版本要的样子。
    async fn swap_network(
        &self,
        old: &Old,
        manifest: &Manifest,
        network: &str,
        progress: &mut Progress,
    ) -> Result<(), InstallError> {
        progress.touched = true;
        self.remove_own_containers(&manifest.id).await?;
        let reach_changed =
            self.engine.network_reach(network).await? != Some(self.network_reach(manifest));
        // 网段不在地址段里也重建：旧版核心让 Docker 分的网段，升级时迁进地址段。
        if reach_changed || self.engine.network_outside_pool(network).await? {
            progress.network_rebuilt = true;
            self.rebuild_network(manifest, network).await?;
        }
        // 局域网和出口的网络也一样：段外的删掉在段内重建（容器已经删了）。出口网络整套核心共用，
        // 重建后出口容器跟着重建，所有出网插件重新接上。
        let lan_network = self.cfg.instance.lan_network(&manifest.id.slug());
        if self.engine.network_outside_pool(&lan_network).await? {
            self.engine.remove_network(&lan_network).await?;
            self.engine
                .create_lan_network(&lan_network, &self.cfg.instance.labels(&manifest.id))
                .await?;
        }
        let egress_network = self.cfg.instance.egress();
        if self.engine.network_outside_pool(&egress_network).await? {
            self.engine
                .remove_container(&self.cfg.instance.egress())
                .await?;
            self.engine.remove_network(&egress_network).await?;
            self.engine
                .create_relay_network(&egress_network, &self.cfg.instance.component_labels())
                .await?;
            let config = self.egress_config()?;
            if let Some(proxy) = &config.url {
                self.sync_relay(proxy, &config, None).await?;
            }
        } else if manifest.permissions.contains(&Permission::NetworkEgress) {
            // 出口网络没动而这个插件要出网：出口容器跟着旧网络一起被删过的话，重新接上。
            self.attach_relay(network).await?;
        }
        self.reattach_core(&old.package.manifest, manifest, network)
            .await?;
        self.register_source(manifest, network).await
    }

    /// 能不能出网是建网络时定的，新版本改了 `network.egress` 只能重建。容器都已经删了。
    pub(in crate::plugin) async fn rebuild_network(
        &self,
        manifest: &Manifest,
        network: &str,
    ) -> Result<(), InstallError> {
        if let Some(core) = &self.cfg.core_container {
            self.engine.disconnect_network(network, core).await?;
        }
        self.detach_relay(network).await?;
        self.detach_mailpit(network).await?;
        self.engine.remove_network(network).await?;
        let labels = self.cfg.instance.labels(&manifest.id);
        self.engine
            .create_network(network, &labels, self.network_reach(manifest))
            .await?;
        if manifest.permissions.contains(&Permission::NetworkEgress) {
            self.attach_relay(network).await?;
        }
        Ok(())
    }

    /// 核心在插件网络里的别名跟着权限走：别名变了要断开重接。
    async fn reattach_core(
        &self,
        from: &Manifest,
        to: &Manifest,
        network: &str,
    ) -> Result<(), InstallError> {
        let Some(core) = &self.cfg.core_container else {
            return Ok(());
        };
        if self.core_aliases_of(from) != self.core_aliases_of(to) {
            self.engine.disconnect_network(network, core).await?;
        }
        self.attach_core(network, core, to).await
    }

    /// 旧容器没动过：收回这次备下的，错误里写上收回失败的。
    async fn undo_prepared(
        &self,
        old: &Old,
        progress: Progress,
        error: InstallError,
    ) -> InstallError {
        let langs = self.owner_languages();
        let id = &old.record.id;
        let mut failures = self.rollback(progress.undo, &langs).await;
        if let Err(e) = self
            .revoke_model_names(id, &progress.added_models, &langs)
            .await
        {
            failures.push(l10n::text(
                &langs,
                "plugin-settings-upgrade-cleanup-model-credentials",
                &[("error", e.to_string().into())],
            ));
        }
        if failures.is_empty() {
            error
        } else {
            error.with_note(
                &langs,
                &l10n::text(
                    &langs,
                    "plugin-settings-upgrade-cleanup-unfinished",
                    &[("failures", l10n::list(&langs, &failures).into())],
                ),
            )
        }
    }

    /// 换过容器之后失败：删掉新版本的一切，用旧记录和旧容器当时的镜像重建旧版本。
    async fn back_to_old(
        &self,
        old: &Old,
        package: &PluginPackage,
        progress: Progress,
    ) -> Result<Deployed, InstallError> {
        let langs = self.owner_languages();
        let id = &old.record.id;
        let manifest = &old.package.manifest;
        let network = self.cfg.instance.network(&old.ctx.slug);
        self.remove_own_containers(id).await?;
        let failures = self.rollback(progress.undo, &langs).await;
        if !failures.is_empty() {
            tracing::error!(plugin = %id, ?failures, "收拾新版本的资源没做完");
        }
        if progress.network_rebuilt {
            self.rebuild_network(manifest, &network).await?;
        }
        self.reattach_core(&package.manifest, manifest, &network)
            .await?;
        self.register_source(manifest, &network).await?;
        self.revoke_model_names(id, &progress.added_models, &langs)
            .await?;
        let mut ctx = old.ctx.clone();
        ctx.core_address = self.core_address_var(manifest, &network).await?;
        ctx.extra_hosts = self.hosts_via_gateway(manifest, &network).await?;
        if let Some(hydra) = &self.oidc {
            match &old.record.oidc_client_secret {
                Some(secret) => {
                    hydra
                        .upsert_client(&client_spec(manifest, &ctx, secret))
                        .await?;
                }
                None if uses_sso(&package.manifest) => hydra.delete_client(id.as_str()).await?,
                None => {}
            }
        }
        let mut plan = render(&old.package, &ctx, &langs)?;
        pin_images(&mut plan, &old.images);
        self.replace_containers(&ctx, plan, Gate::StartPeriod)
            .await?;
        // 对账要取宿主信息、起检查容器，链很深：同样放堆上。
        Box::pin(self.reconcile_running(&old.package, &old.record)).await
    }

    /// 新版本不要了的：记录已经写好，失败只记日志，主人卸载时一并清掉。
    async fn drop_unused(&self, old: &Old, package: &PluginPackage, langs: &Languages) {
        let manifest = &package.manifest;
        let id = &manifest.id;
        let was = &old.package.manifest;
        if let (true, false, Some(hydra)) = (uses_sso(was), uses_sso(manifest), &self.oidc)
            && let Err(e) = hydra.delete_client(id.as_str()).await
        {
            tracing::warn!(plugin = %id, error = %e, "删新版本不用的 OIDC 客户端失败");
        }
        if !manifest.permissions.contains(&Permission::Models)
            && let Err(e) = self.revoke_models(id, langs).await
        {
            tracing::warn!(plugin = %id, error = %e, "吊销新版本不用的模型网关凭据失败");
        }
        if is_bot(was)
            && !is_bot(manifest)
            && let Err(e) = self.deactivate_bots(id, false).await
        {
            tracing::warn!(plugin = %id, error = %e, "吊销新版本不用的 Bot 设备失败");
        }
        let lan_network = self.cfg.instance.lan_network(&id.slug());
        let used = self
            .lan_grant_on(package, false, Vec::new(), Vec::new())
            .network
            .is_some();
        if !used
            && self
                .engine
                .network_reach(&lan_network)
                .await
                .is_ok_and(|r| r.is_some())
            && let Err(e) = self.engine.remove_network(&lan_network).await
        {
            tracing::warn!(plugin = %id, error = %e, "删新版本不用的局域网网络失败");
        }
        if manifest
            .provides
            .as_ref()
            .is_some_and(|p| p.models.is_some())
        {
            for (plugin, error) in self.reconcile_models().await {
                tracing::warn!(%plugin, %error, "模型网关升级后对账失败");
            }
        }
    }

    /// 系统插件随核心升级（插件规范的“预装插件”）：目录里的版本比装着的新时按同样的步骤升级，装核心就是对它的确认。启动时
    /// 等核心自己的升级结束才走到这里（见 `main.rs` 的预装）。失败时旧版本继续跑，如实列出，下次启动再试。
    pub(in crate::plugin) async fn upgrade_bundled(&self, package: PluginPackage) {
        let manifest = &package.manifest;
        let id = manifest.id.clone();
        if manifest.kind != PluginKind::System {
            return;
        }
        let installed = match self.store.load(&id) {
            Ok(Some(record)) => record,
            Ok(None) => return,
            Err(e) => {
                tracing::error!(plugin = %id, error = %e, "读系统插件的记录失败，没有升级");
                return;
            }
        };
        let version = manifest.version.clone();
        if !newer(&version, &installed.version) {
            return;
        }
        let granted = manifest.permissions.clone();
        if let Err(e) = self
            .upgrade_checked(
                package,
                &granted,
                &installed.enabled_permissions,
                installed.lan_webui,
                installed.settings.clone(),
            )
            .await
        {
            let from = installed.version;
            tracing::error!(plugin = %id, error = %e, "系统插件升级失败");
            let reason = l10n::text(
                &self.owner_languages(),
                "plugin-settings-upgrade-bundled-failed",
                &[
                    ("version", version.as_str().into()),
                    ("from", from.as_str().into()),
                    ("error", e.to_string().into()),
                ],
            );
            self.upgrades.bundled_failed(&id, reason);
            self.changed();
        }
    }
}
