//! 换容器：读回旧版本的上下文和镜像，备好新版本要的资源，删掉旧容器、重建网络、接回核心。

use std::collections::HashMap;
use std::sync::Arc;

use tokio::time::Instant;
use wonton_protocol::{BotKeeper, InstallStage};

use super::super::bots::is_bot;
use super::super::lifecycle::volumes::{VolumeAction, volume_action};
use super::super::lifecycle::{Grants, client_spec};
use super::super::manager::{Deployed, InstallError, PluginManager};
use super::super::manifest::{Manifest, ManifestExt, Permission, PluginId};
use super::super::package::PluginPackage;
use super::super::proxy_auth::ProxyAuth;
use super::super::render::render;
use super::super::secrets::{self, CollectMissing};
use super::super::store::{EnabledPermission, InstalledRecord};
use super::super::upstreams::uses_sso;
use super::installed_package;
use super::run::{Old, Prepared, Progress};
use crate::secret::random_token;

impl PluginManager {
    /// 读回旧版本的上下文和令牌：读不到就报错，容器还没动。
    pub(super) async fn read_old(&self, id: &PluginId) -> Result<Old, InstallError> {
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
    #[allow(clippy::too_many_lines)] // 升级就是把安装各步按原顺序重放一遍，拆开只会把顺序藏进多函数。
    pub(super) async fn replace_version(
        &self,
        old: &Old,
        package: &PluginPackage,
        granted: &[Permission],
        enabled: &[EnabledPermission],
        grants: Grants,
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
            devices,
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
        self.sync_bot_web_clients(manifest, &ctx).await?;
        self.rewrite_image_healthchecks(&ctx, &mut plan).await?;
        let wait = super::super::health_gate::deadline(&plan, ctx.install_limit, Instant::now());
        self.stage(&manifest.id, InstallStage::Starting);
        let health = Box::pin(self.start_containers(
            Some(manifest),
            &plan,
            Some(&manifest.id),
            &mut progress.undo,
            wait,
            &langs,
        ))
        .await?;
        let (upstream, models, mcp) = self
            .deploy_upstreams(
                manifest,
                &network,
                &mut secrets,
                wait,
                CollectMissing::Fail,
                &langs,
            )
            .await?;
        let firewalled = self
            .host_webui_firewalled(manifest, &plan.relays, &network)
            .await;
        if let Some((_, _, bot)) = &progress.undo.bot {
            progress.undo.handed_over = self.hand_over_home_room(&self.cfg.owner, bot).await?;
        }
        self.save_upgraded_record(InstalledRecord {
            lifecycle: super::super::recovery::running(),
            id: manifest.id.clone(),
            version: manifest.version.clone(),
            source: package.source.clone(),
            files: package.files().clone(),
            granted_permissions: super::super::grant::recorded_grant(manifest, granted),
            enabled_permissions: enabled.to_vec(),
            devices,
            limits: old.record.limits,
            lan_webui: ctx.lan.as_ref().is_some_and(|lan| lan.webui),
            relays: plan.relays.clone(),
            firewalled,
            proxied: ctx.egress.is_some(),
            bundled: false,
            oidc_client_secret: oidc_secret,
            installed_at: old.record.installed_at,
            entry_label: None,
            clients_entries: Default::default(),
            setup_done_ever: false,
            origin: package.origin.clone(),
            // 归属跟着插件走：Agent 更新自己写的插件后还是它写的；App 里经文件或来源升级的清掉（各路径自己做）。
            written_by: old.record.written_by.clone(),
            ask_every_update: old.record.ask_every_update,
            settings: ctx.settings.clone().into_iter().collect(),
            settings_changed_ms: old.record.settings_changed_ms,
            static_files: Default::default(),
            hosted: Vec::new(),
            gpu_vendor: ctx.gpu.as_ref().map(|gpu| gpu.vendor),
        })
        .await?;
        self.refresh_bot_web_routes(manifest).await?;
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
            clients: self.clients_upstream(manifest, &plan).await?,
        })
    }

    /// 旧容器还在跑时能做的都做了：渲染、凭据、拉镜像、目录和卷。这一步失败时旧版本不受影响。
    async fn prepare_new(
        &self,
        old: &Old,
        package: &PluginPackage,
        enabled: &[EnabledPermission],
        grants: Grants,
        settings: super::super::settings::Values,
        progress: &mut Progress,
    ) -> Result<Prepared, InstallError> {
        let manifest = &package.manifest;
        self.ensure_bot_web_registration(manifest).await?;
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
        ctx.gpu = grants.gpu;
        ctx.lan = grants.lan;
        ctx.devices = Some(grants.devices);
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
            self.prepare_bot_dependencies(
                manifest,
                super::super::bot_services::DEFAULT_BOT,
                &langs,
                None,
            )
            .await?;
            self.fill_default_bot_vars(manifest, &bot, &mut ctx)?;
        } else {
            self.fill_bot_vars(package, &mut ctx, &langs).await?;
        }
        self.fill_bot_web_vars(manifest, &mut ctx)?;
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
            devices: grants.device_record,
            secrets,
            proxy,
            oidc_secret,
            pulled: plan,
        })
    }

    /// 删掉这个插件所有的容器，包括新版本不要了的服务的。
    pub(super) async fn remove_own_containers(&self, id: &PluginId) -> Result<(), InstallError> {
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
    pub(super) async fn reattach_core(
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
}
