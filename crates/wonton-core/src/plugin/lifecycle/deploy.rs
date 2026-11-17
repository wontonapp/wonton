//! 安装部署主流程：渲染部署计划、建资源、起容器、写安装记录，失败时交回滚。

use std::sync::Arc;

use tokio::time::Instant;
use wonton_protocol::InstallStage;

use super::super::languages::Languages;
use super::super::manager::{Deployed, InstallError, PluginManager};
use super::super::manifest::{Manifest, Permission};
use super::super::models::ModelsRoute;
use super::super::package::PluginPackage;
use super::super::render::{Deployment, InstallContext, render};
use super::super::secrets::Secrets;
use super::super::store::EnabledPermission;
use super::super::upstreams::{Endpoint, McpTarget};
use super::Grants;
use super::Undo;
use super::client_spec;

impl PluginManager {
    /// 部署插件并写下安装记录，返回核心访问插件网页的地址。任一步失败（包括写记录）都回滚本次创建的
    /// 资源：没有记录的容器、网络和 OIDC 客户端，核心重启后就没人管了。`gpu` 是交给 `gpu.services` 的显卡，`lan` 是局域网端口开在哪里。
    #[allow(clippy::too_many_arguments)] // 装插件要的几样各管各的来源，打包反而看不清哪来的。
    pub(in crate::plugin) async fn deploy_retained(
        &self,
        package: &PluginPackage,
        granted: &[Permission],
        enabled: &[EnabledPermission],
        grants: Grants,
        settings: std::collections::BTreeMap<String, serde_json::Value>,
        langs: &Languages,
    ) -> Result<(Deployed, Undo), InstallError> {
        let _allocating = self.bot_allocation.lock().await;
        self.reserve_manifest_bot_web(&package.manifest)?;
        let mut undo = Undo {
            web_plugin: Some(package.manifest.id.clone()),
            ..Default::default()
        };
        // 同 installation.rs 的 install_checked：deploy 子树的状态机搬上堆，install 链默认栈才够深。
        match Box::pin(self.deploy_inner(
            package, granted, enabled, grants, settings, &mut undo, langs,
        ))
        .await
        {
            Ok(deployed) => {
                undo.final_stage = Some(self.install_stage(&package.manifest.id));
                Ok((deployed, undo))
            }
            Err(error) => {
                let id = &package.manifest.id;
                let stage = self.install_stage(id);
                tracing::warn!(plugin = %id, ?stage, kind = error.kind(), error = %error,
                    cleanup_result = "pending", "Plugin installation failed; starting rollback");
                undo.keep_bot_data();
                let failures = self.rollback(undo, langs).await;
                self.installation_failed(id, &error, stage, failures.clone(), langs);
                Err(self.install_rollback_error(error, failures, langs))
            }
        }
    }

    #[allow(clippy::too_many_arguments)] // 装插件要的几样各管各的来源，打包反而看不清哪来的。
    async fn deploy_inner(
        &self,
        package: &PluginPackage,
        granted: &[Permission],
        enabled: &[EnabledPermission],
        grants: Grants,
        settings: std::collections::BTreeMap<String, serde_json::Value>,
        undo: &mut Undo,
        langs: &Languages,
    ) -> Result<Deployed, InstallError> {
        let manifest = &package.manifest;
        let (mut ctx, secret, mut secrets, proxy) = self
            .prepare_install(
                package,
                enabled,
                (grants.gpu, grants.lan),
                settings,
                undo,
                langs,
            )
            .await?;
        ctx.devices = Some(grants.devices);
        let device_record = grants.device_record;
        let mut plan = render(package, &ctx, langs)?;
        self.prepare_images(&mut plan, Some(&manifest.id), langs)
            .await?;
        self.stage(&manifest.id, InstallStage::Preparing);
        self.prepare_dirs(&ctx, &plan.dirs, undo, langs)?;
        // 中继套接字目录不在应用数据目录下，不属于任何一个插件：装之前单独建、交给 PUID。
        if !plan.relays.is_empty() {
            self.prepare_relay_dir(langs)?;
        }
        // create_networks 里经 attach_relay→sync_relay 到 bollard 的 inspect 反序列化是整条
        // install 链的栈顶；box 上堆后默认 2 MiB 测试线程栈才装得下（egress_plugin_docker 的栈溢出）。
        Box::pin(self.create_networks(manifest, &plan, &ctx, undo)).await?;
        // 容器启动前登记：安装过程中 Bot 就要经核心连 Matrix，健康检查才过得了。
        self.register_source(manifest, &plan.network).await?;
        undo.source = Some(manifest.id.clone());
        if let Some(core) = &self.cfg.core_container {
            self.attach_core(&plan.network, core, manifest).await?;
            undo.core_attached = true;
        }
        self.fill_network_vars(manifest, &plan, &mut ctx).await?;
        let pulled = plan;
        let mut plan = render(package, &ctx, langs)?;
        super::super::registry_mirror::use_pulled_images(&mut plan, pulled);
        self.create_volumes(&plan, manifest, undo, langs).await?;
        if let Some(secret) = &secret {
            self.hydra(langs)?
                .upsert_client(&client_spec(manifest, &ctx, secret))
                .await?;
            undo.oidc_client = Some(manifest.id.to_string());
        }
        self.sync_bot_web_clients(manifest, &ctx).await?;
        self.rewrite_image_healthchecks(&ctx, &mut plan).await?;
        let deadline =
            super::super::health_gate::deadline(&plan, ctx.install_limit, Instant::now());
        self.stage(&manifest.id, InstallStage::Starting);
        let health = Box::pin(self.start_containers(
            Some(manifest),
            &plan,
            Some(&manifest.id),
            undo,
            deadline,
            langs,
        ))
        .await?;
        let (upstream, models, mcp) = self
            .started_upstreams(manifest, &plan, &mut secrets, undo, deadline, langs)
            .await?;
        let firewalled = self
            .host_webui_firewalled(manifest, &plan.relays, &plan.network)
            .await;
        self.save_installed(
            package,
            granted,
            enabled,
            &ctx,
            &plan,
            secret,
            firewalled,
            device_record,
        )
        .await?;
        undo.record = Some(manifest.id.clone());
        self.refresh_bot_web_routes(&package.manifest).await?;
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
            hosted: self
                .hosted_upstreams(&package.manifest, &plan.network)
                .await?,
            clients: self.clients_upstream(manifest, &plan).await?,
        })
    }

    /// 安装成功后的记录：权限、局域网开关、代理和密钥都在这一步落盘。
    #[allow(clippy::too_many_arguments)]
    async fn save_installed(
        &self,
        package: &PluginPackage,
        granted: &[Permission],
        enabled: &[EnabledPermission],
        ctx: &InstallContext,
        plan: &Deployment,
        oidc_secret: Option<String>,
        firewalled: bool,
        devices: Vec<super::super::store::InstalledDevice>,
    ) -> Result<(), InstallError> {
        let manifest = &package.manifest;
        self.store.save(&super::super::store::InstalledRecord {
            lifecycle: super::super::recovery::running(),
            id: manifest.id.clone(),
            version: manifest.version.clone(),
            source: package.source.clone(),
            files: package.files().clone(),
            granted_permissions: granted.to_vec(),
            enabled_permissions: enabled.to_vec(),
            devices,
            limits: Default::default(),
            lan_webui: ctx.lan.as_ref().is_some_and(|lan| lan.webui),
            relays: plan.relays.clone(),
            firewalled,
            proxied: ctx.egress.is_some(),
            oidc_client_secret: oidc_secret,
            installed_at: super::super::manager::now_secs(),
            entry_label: self.install_entry_label(&manifest.id)?,
            clients_entries: Default::default(),
            setup_done_ever: false,
            origin: package.origin.clone(),
            written_by: None,
            ask_every_update: false,
            settings: ctx.settings.clone().into_iter().collect(),
            settings_changed_ms: None,
            static_files: Default::default(),
            hosted: Vec::new(),
            gpu_vendor: ctx.gpu.as_ref().map(|gpu| gpu.vendor),
        })?;
        Ok(())
    }

    /// 容器起来之后的收尾：收存密钥和入口（见 [`PluginManager::deploy_upstreams`]），把 Bot 的房间交给核心。
    async fn started_upstreams(
        &self,
        manifest: &Manifest,
        plan: &Deployment,
        secrets: &mut Secrets,
        undo: &mut Undo,
        deadline: Instant,
        langs: &Languages,
    ) -> Result<(Option<Endpoint>, Option<ModelsRoute>, Option<McpTarget>), InstallError> {
        let upstreams = self
            .deploy_upstreams(manifest, &plan.network, secrets, deadline, langs)
            .await?;
        if let Some((_, _, bot)) = &undo.bot {
            undo.handed_over = self.hand_over_home_room(&self.cfg.owner, bot).await?;
        }
        Ok(upstreams)
    }
}
