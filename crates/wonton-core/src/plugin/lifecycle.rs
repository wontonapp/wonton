//! 部署、恢复与卸载：把部署计划交给 Docker，失败时按相反顺序回滚。

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tokio::time::Instant;
use wonton_protocol::InstallStage;

use super::compose::DependCondition;
use super::configs;
use super::configs_store;
use super::core_address;
use super::docker::{LabeledResources, NetworkReach};
use super::gpu::GpuGrant;
use super::health::{self, Readiness, Target};
use super::hostdir::{self, Owner};
use super::lan::LanGrant;
use super::languages::Languages;
use super::leftovers::name_taken;
use super::manager::{Deployed, Health, InstallError, PluginManager, UninstallReport};
use super::manifest::{Manifest, Permission, PluginId, Sso};
use super::models::ModelsRoute;
use super::names::{Instance, LABEL_PLUGIN};
use super::package::PluginPackage;
use super::proxy_auth::ProxyAuth;
use super::render::{Deployment, InstallContext, render};
use super::secrets::{self, Secrets};
use super::store::{EnabledPermission, InstalledRecord};
use super::upstreams::Endpoint;
use super::upstreams::{McpTarget, core_aliases, uses_sso};
use super::vars::{OidcVars, OwnerVars};
use crate::config::PublicUrls;
use crate::l10n;
use crate::matrix::ProvisionedBot;
use crate::oidc::{ClientSpec, Hydra};
use crate::secret::random_token;

pub(super) mod rollback;

/// 插件管理器需要的那部分核心配置。
#[derive(Debug, Clone)]
pub struct LifecycleConfig {
    pub public: PublicUrls,
    pub data_dir: PathBuf,
    pub apps_dir: PathBuf,
    pub shares_dir: PathBuf,
    pub core_container: Option<String>,
    pub tz: String,
    pub puid: u32,
    pub pgid: u32,
    pub install_timeout: Duration,
    /// 主人的用户名，也是主人的 Matrix 用户名（localpart）。
    pub owner: String,
    /// 主人的显示名和邮箱，随时会改。邮箱注入给要身份的插件。
    pub profile: Arc<crate::owner::OwnerStore>,
    /// 核心网关监听的端口：插件经核心在插件网络里的别名（如 `matrix.wonton.internal`）访问它。
    pub gateway_port: u16,
    /// 核心给插件容器设置的 systemd slice；空值表示不设置父 cgroup。
    pub plugin_slice: Option<String>,
    /// 核心的实例名（`WONTON_INSTANCE`），决定插件的资源名和标签。
    pub instance: Instance,
}

/// 本次安装已经创建的资源，失败时据此回滚。
#[derive(Default)]
pub(super) struct Undo {
    containers: Vec<String>,
    pub(super) volumes: Vec<String>,
    network: Option<String>,
    /// 发布局域网端口的网络（见 [`super::lan`]）。
    pub(super) lan_network: Option<String>,
    core_attached: bool,
    oidc_client: Option<String>,
    app_dir_created: Option<PathBuf>,
    /// 本次安装之前插件的密钥（`None` 表示原来没有），回滚时恢复成这样。
    secrets_before: Option<(PluginId, Option<Secrets>)>,
    /// 本次安装代 Bot 登录的设备，回滚时吊销（账号和主房间保留给下次安装）。
    pub(super) bot: Option<(PluginId, String, ProvisionedBot)>,
    /// 本次安装把主房间交给了主人（主人原来不在里面），回滚时收回。
    pub(super) handed_over: bool,
    /// 已登记插件网络的地址段，回滚时注销。
    source: Option<PluginId>,
    /// 本次安装在模型网关里签发了凭据的插件，回滚时吊销。
    models: Option<PluginId>,
}

/// 插件声明的命名卷怎么处理。
#[derive(Debug, PartialEq, Eq)]
pub(super) enum VolumeAction {
    Create,
    Reuse,
}

/// Docker 创建同名卷时直接返回已有的卷，所以先看它属于谁：不存在就新建（安装失败时回滚删除）；
/// 属于本插件（上次不删数据卸载后保留的）就复用，回滚时不删；属于其他插件或不是核心创建的，拒绝安装。
pub(super) fn volume_action(
    volume: &str,
    labels: Option<&HashMap<String, String>>,
    plugin: &PluginId,
    langs: &Languages,
) -> Result<VolumeAction, InstallError> {
    let Some(labels) = labels else {
        return Ok(VolumeAction::Create);
    };
    if labels.get(LABEL_PLUGIN).map(String::as_str) == Some(plugin.as_str()) {
        Ok(VolumeAction::Reuse)
    } else {
        Err(name_taken(
            &l10n::text(langs, "plugin-install-word-volume", &[]),
            volume,
            labels,
            langs,
        ))
    }
}

impl PluginManager {
    pub(super) fn hydra(&self, langs: &Languages) -> Result<&Hydra, InstallError> {
        self.oidc.as_ref().ok_or_else(|| {
            InstallError::OidcUnavailable(l10n::text(
                langs,
                "plugin-install-oidc-unconfigured",
                &[],
            ))
        })
    }

    /// 部署插件并写下安装记录，返回核心访问插件网页的地址。任一步失败（包括写记录）都回滚本次创建的
    /// 资源：没有记录的容器、网络和 OIDC 客户端，核心重启后就没人管了。`gpu` 是交给 `gpu.services` 的显卡，`lan`
    /// 是局域网端口开在哪里。
    pub(super) async fn deploy(
        &self,
        package: &PluginPackage,
        granted: &[Permission],
        enabled: &[EnabledPermission],
        gpu: Option<GpuGrant>,
        lan: Option<LanGrant>,
        langs: &Languages,
    ) -> Result<Deployed, InstallError> {
        let mut undo = Undo::default();
        match self
            .deploy_inner(package, granted, enabled, (gpu, lan), &mut undo, langs)
            .await
        {
            Ok(deployed) => Ok(deployed),
            Err(error) => {
                let failures = self.rollback(undo, langs).await;
                if failures.is_empty() {
                    Err(error)
                } else {
                    Err(error.with_note(
                        langs,
                        &l10n::text(
                            langs,
                            "plugin-install-rollback-unfinished",
                            &[("failures", l10n::list(langs, &failures).into())],
                        ),
                    ))
                }
            }
        }
    }

    /// 安装时代默认 Bot 登录（归属人是主人），失败时回滚吊销设备。没有 Bot 的插件返回 `None`。
    /// 按计划建卷；已存在的卷不动（见 [`volume_action`]）。
    async fn create_volumes(
        &self,
        plan: &Deployment,
        manifest: &Manifest,
        undo: &mut Undo,
        langs: &Languages,
    ) -> Result<(), InstallError> {
        for volume in &plan.volumes {
            let labels = self.engine.volume_labels(volume).await?;
            if volume_action(volume, labels.as_ref(), &manifest.id, langs)? == VolumeAction::Create
            {
                self.engine.create_volume(volume, &plan.labels).await?;
                undo.volumes.push(volume.clone());
            }
        }
        Ok(())
    }

    async fn deploy_inner(
        &self,
        package: &PluginPackage,
        granted: &[Permission],
        enabled: &[EnabledPermission],
        (gpu, lan): (Option<GpuGrant>, Option<LanGrant>),
        undo: &mut Undo,
        langs: &Languages,
    ) -> Result<Deployed, InstallError> {
        let manifest = &package.manifest;
        self.stage(&manifest.id, InstallStage::Preparing);
        self.clear_leftovers(&manifest.id).await?;
        let secret = uses_sso(manifest).then(random_token);
        let mut secrets = self.prepare_secrets(manifest, undo)?;
        let generated =
            secrets::generated_values(manifest, &secrets, langs).map_err(InstallError::Storage)?;
        let proxy =
            ProxyAuth::resolve(manifest, &generated, langs).map_err(InstallError::Storage)?;
        let effective = manifest.effective_permissions(&super::store::permission_set(enabled));
        let mut ctx = self
            .install_context(manifest, secret.as_deref(), generated, &effective, langs)
            .await?;
        ctx.gpu = gpu;
        ctx.lan = lan;
        self.check_network_names(&ctx).await?;
        let default_bot = self.provision_default_bot(package, undo).await?;
        if let Some(models) = self.provision_models(package, langs).await? {
            ctx.models = Some(models);
            undo.models = Some(manifest.id.clone());
        }
        if let Some(bot) = &default_bot {
            self.fill_default_bot_vars(manifest, bot, &mut ctx);
        }
        let mut plan = render(package, &ctx, langs)?;
        self.prepare_images(&mut plan, Some(&manifest.id), langs)
            .await?;
        self.stage(&manifest.id, InstallStage::Preparing);
        self.prepare_dirs(&ctx, &plan.dirs, undo, langs)?;
        self.create_networks(manifest, &plan, &ctx, undo).await?;
        // 容器启动前登记：安装过程中 Bot 就要经核心连 Matrix，健康检查才过得了。
        self.register_source(manifest, &plan.network).await?;
        undo.source = Some(manifest.id.clone());
        if let Some(core) = &self.cfg.core_container {
            self.attach_core(&plan.network, core, manifest).await?;
            undo.core_attached = true;
        }
        // 网络建好才知道的：核心在网络里的地址和网关。
        ctx.core_address = self.core_address_var(manifest, &plan.network).await?;
        ctx.extra_hosts = self.hosts_via_gateway(manifest, &plan.network).await?;
        let pulled = plan;
        let mut plan = render(package, &ctx, langs)?;
        super::registry_mirror::use_pulled_images(&mut plan, pulled);
        self.create_volumes(&plan, manifest, undo, langs).await?;
        if let Some(secret) = &secret {
            self.hydra(langs)?
                .upsert_client(&client_spec(manifest, &ctx, secret))
                .await?;
            undo.oidc_client = Some(manifest.id.to_string());
        }
        self.rewrite_image_healthchecks(&ctx, &mut plan).await?;
        let deadline =
            super::health_gate::deadline(&plan, self.cfg.install_timeout, Instant::now());
        self.stage(&manifest.id, InstallStage::Starting);
        let health = self
            .start_containers(&plan, Some(&manifest.id), undo, deadline, langs)
            .await?;
        let (upstream, models, mcp) = self
            .started_upstreams(manifest, &plan, &mut secrets, undo, deadline, langs)
            .await?;
        let firewalled = self
            .host_webui_firewalled(manifest, &plan.relays, &plan.network)
            .await;
        self.store.save(&InstalledRecord {
            id: manifest.id.clone(),
            version: manifest.version.clone(),
            source: package.source.clone(),
            files: package.files().clone(),
            granted_permissions: granted.to_vec(),
            enabled_permissions: enabled.to_vec(),
            limits: Default::default(),
            lan_webui: ctx.lan.as_ref().is_some_and(|lan| lan.webui),
            relays: plan.relays.clone(),
            firewalled,
            proxied: ctx.egress.is_some(),
            oidc_client_secret: secret,
            installed_at: super::manager::now_secs(),
            origin: package.origin.clone(),
            written_by: None,
            ask_every_update: false,
            family: false,
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
        })
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

    /// 建插件网络，以及局域网端口要用的网络（见 [`super::lan`]）。
    async fn create_networks(
        &self,
        manifest: &Manifest,
        plan: &Deployment,
        ctx: &InstallContext,
        undo: &mut Undo,
    ) -> Result<(), InstallError> {
        self.engine
            .create_network(&plan.network, &plan.labels, self.network_reach(manifest))
            .await?;
        undo.network = Some(plan.network.clone());
        if ctx.egress.is_some() {
            self.attach_relay(&plan.network).await?;
        }
        if let Some(network) = ctx.lan.as_ref().and_then(|l| l.network.clone()) {
            self.engine
                .create_lan_network(&network, &plan.labels)
                .await?;
            undo.lan_network = Some(network);
        }
        Ok(())
    }

    /// 容器起来之后收存插件写下的密钥（`collect` 项），变了就保存。Bot 服务上的按 Bot 各一份。
    /// 读出插件的密钥（上次不删数据卸载时保留的也在），给还没有值的 `generate` 项生成并保存。
    pub(super) fn prepare_secrets(
        &self,
        manifest: &Manifest,
        undo: &mut Undo,
    ) -> Result<Secrets, InstallError> {
        if manifest.secrets.is_empty() {
            return Ok(Secrets::new());
        }
        let store = self.store.secrets();
        let before = store.load(&manifest.id)?;
        let mut secrets = before.clone().unwrap_or_default();
        undo.secrets_before = Some((manifest.id.clone(), before));
        if secrets::generate_missing(manifest, &mut secrets) {
            store.save(&manifest.id, &secrets)?;
        }
        Ok(secrets)
    }

    /// 注入给 SSO 插件的端点：签发方和后台端点在插件网络里，授权端点给浏览器（ADR-0063）。
    async fn oidc_vars(
        &self,
        manifest: &Manifest,
        secret: Option<&str>,
        langs: &Languages,
    ) -> Result<Option<OidcVars>, InstallError> {
        let Some(secret) = secret else {
            return Ok(None);
        };
        let public = &self.cfg.public;
        let discovery = self
            .hydra(langs)?
            .resolved_discovery(
                &public.origin(&public.auth_host()),
                &public.auth_internal_origin(),
            )
            .await?;
        Ok(Some(OidcVars {
            issuer: discovery.issuer,
            client_id: manifest.id.to_string(),
            client_secret: secret.to_owned(),
            auth_url: discovery.authorization_endpoint,
            token_url: discovery.token_endpoint,
            userinfo_url: discovery.userinfo_endpoint,
        }))
    }

    pub(super) async fn install_context(
        &self,
        manifest: &Manifest,
        secret: Option<&str>,
        generated: BTreeMap<String, String>,
        granted: &[Permission],
        langs: &Languages,
    ) -> Result<InstallContext, InstallError> {
        if self.cfg.profile.email().is_none() && super::manifest_oidc::needs_owner_email(manifest) {
            return Err(InstallError::OwnerProfileIncomplete(
                crate::owner::no_email(langs),
            ));
        }
        let public = &self.cfg.public;
        let slug = manifest.id.slug();
        let app_url = manifest
            .webui
            .as_ref()
            .map(|_| public.origin(&public.plugin_host(&slug)));
        let oidc = self.oidc_vars(manifest, secret, langs).await?;
        let shares = manifest
            .permissions
            .iter()
            .filter_map(|p| match p {
                Permission::StorageShare { name, .. } => {
                    Some((name.clone(), self.cfg.shares_dir.join(name)))
                }
                _ => None,
            })
            .collect::<BTreeMap<_, _>>();
        if !shares.is_empty() {
            fs::create_dir_all(&self.cfg.shares_dir).map_err(|e| {
                InstallError::Storage(l10n::text(
                    langs,
                    "plugin-install-shared-folder-mkdir-failed",
                    &[
                        ("dir", self.cfg.shares_dir.display().to_string().into()),
                        ("error", e.to_string().into()),
                    ],
                ))
            })?;
        }
        for path in shares.values() {
            hostdir::ensure_owned_dir(path, self.owner()).map_err(|e| {
                InstallError::Storage(l10n::text(
                    langs,
                    "plugin-install-shared-folder-prepare-failed",
                    &[("error", e.to_string().into())],
                ))
            })?;
        }
        Ok(InstallContext {
            app_data: self.cfg.apps_dir.join(&slug),
            config_store: self.cfg.data_dir.join("core").join("configs"),
            slug,
            instance: self.cfg.instance.clone(),
            shares,
            app_url,
            oidc,
            owner: manifest.gets_owner_identity().then(|| OwnerVars {
                name: self.cfg.owner.clone(),
                email: self.cfg.profile.email(),
            }),
            matrix: None,
            bots: Default::default(),
            models: None,
            mcp_url: manifest.permissions.contains(&Permission::Tools).then(|| {
                let port = self.cfg.gateway_port;
                format!("http://{}:{port}/mcp", crate::config::PLUGIN_MCP_HOST)
            }),
            notify_url: manifest.permissions.contains(&Permission::Notify).then(|| {
                let port = self.cfg.gateway_port;
                format!(
                    "http://{}:{port}/notices",
                    crate::config::PLUGIN_NOTIFY_HOST
                )
            }),
            secrets: generated,
            tz: self.cfg.tz.clone(),
            puid: self.cfg.puid,
            pgid: self.cfg.pgid,
            extra_hosts: Vec::new(),
            gpu: None,
            lan: None,
            egress: self.egress_vars(manifest)?,
            core_address: (manifest.webui.as_ref().is_some_and(|w| w.sso == Sso::Proxy))
                .then(|| core_address::PLACEHOLDER.to_owned()),
            limits: Default::default(),
            memory_total: self.memory_total().await,
            host_name: self.host_name().await,
            cgroup_parent: self.cfg.plugin_slice.clone(),
            granted: granted.to_vec(),
        })
    }

    /// 插件网络能连到哪里：声明了 `network.egress` 才能出网。不能出网时，核心在容器里就连宿主也隔开（核心
    /// 加入插件网络访问插件）；核心直接跑在宿主上时要经网桥访问插件，只能用普通的内部网络。
    pub(super) fn network_reach(&self, manifest: &Manifest) -> NetworkReach {
        if manifest.permissions.contains(&Permission::NetworkEgress) {
            NetworkReach::Egress
        } else if self.cfg.core_container.is_some() {
            NetworkReach::Isolated
        } else {
            NetworkReach::Internal
        }
    }

    /// 核心直接跑在宿主上时，插件经自己网络的网关（宿主上网桥的地址）访问核心的别名（[`core_aliases`]）：
    /// 内部网络没有默认路由，`host-gateway`（另一个网桥的地址）走不通。返回要写进 `/etc/hosts` 的条目。
    pub(super) async fn hosts_via_gateway(
        &self,
        manifest: &Manifest,
        network: &str,
    ) -> Result<Vec<String>, InstallError> {
        let aliases = core_aliases(&self.cfg.public, manifest);
        if self.cfg.core_container.is_some() || aliases.is_empty() {
            return Ok(Vec::new());
        }
        let gateway = self.engine.network_gateway(network).await?;
        Ok(aliases
            .into_iter()
            .map(|alias| format!("{alias}:{gateway}"))
            .collect())
    }

    /// 登记插件网络的地址段，网关据此认出插件的请求（见 [`super::sources`]）。
    pub(super) async fn register_source(
        &self,
        manifest: &Manifest,
        network: &str,
    ) -> Result<(), InstallError> {
        let subnets = self.engine.network_subnets(network).await?;
        self.sources
            .insert(&manifest.id, subnets, manifest.permissions.clone());
        Ok(())
    }

    /// 主人的用户（PUID:PGID）：共享文件夹、应用数据目录本身和中间各级目录都交给它。
    pub(super) fn owner(&self) -> Owner {
        Owner {
            uid: self.cfg.puid,
            gid: self.cfg.pgid,
        }
    }

    /// 绑定挂载的源目录由核心预先创建并交给挂载它的服务的属主，否则 Docker 会以 root 身份创建。
    /// 逐级检查、不跟随软链接：应用数据目录里可能有上次安装的插件留下的软链接。
    pub(super) fn prepare_dirs(
        &self,
        ctx: &InstallContext,
        dirs: &BTreeMap<PathBuf, Owner>,
        undo: &mut Undo,
        langs: &Languages,
    ) -> Result<(), InstallError> {
        if dirs.is_empty() {
            return Ok(());
        }
        let storage = |e: &dyn std::fmt::Display| {
            InstallError::Storage(l10n::text(
                langs,
                "plugin-install-app-data-prepare-failed",
                &[("error", e.to_string().into())],
            ))
        };
        fs::create_dir_all(&self.cfg.apps_dir).map_err(|e| storage(&e))?;
        if fs::symlink_metadata(&ctx.app_data)
            .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
        {
            undo.app_dir_created = Some(ctx.app_data.clone());
        }
        hostdir::prepare(&ctx.app_data, self.owner(), dirs).map_err(|e| storage(&e))
    }

    pub(super) async fn start_containers(
        &self,
        plan: &Deployment,
        id: Option<&PluginId>,
        undo: &mut Undo,
        deadline: Instant,
        langs: &Languages,
    ) -> Result<Health, InstallError> {
        for container in &plan.containers {
            let pack_failed = |e: &dyn std::fmt::Display| {
                InstallError::Invalid(l10n::text(
                    langs,
                    "plugin-install-configs-pack-failed",
                    &[
                        ("service", container.name.as_str().into()),
                        ("error", e.to_string().into()),
                    ],
                ))
            };
            health::wait_for_all(&self.engine, &container.depends_on, deadline, langs).await?;
            // 只读根的服务先经一次性容器把配置文件写进核心数据目录：单文件绑定挂载的源在创建容器
            // 时就必须存在，归档接口写不进只读根（见 configs_store 的模块注释）。
            if let Some(dir) = &container.store_dir {
                let archive =
                    configs_store::archive(&container.files).map_err(|e| pack_failed(&e))?;
                configs_store::seed(
                    &self.engine,
                    &container.name,
                    &container.image,
                    dir,
                    archive,
                    &plan.labels,
                )
                .await?;
            }
            self.engine
                .create_container(&container.name, container.body.clone())
                .await?;
            undo.containers.push(container.name.clone());
            if !container.files.is_empty() && container.store_dir.is_none() {
                let archive = configs::archive(&container.files).map_err(|e| pack_failed(&e))?;
                self.engine
                    .upload_archive(&container.name, "/", archive)
                    .await?;
            }
            self.engine.start_container(&container.name).await?;
        }
        if let Some(id) = id {
            self.stage(id, InstallStage::WaitingForBot);
        }
        // 没人依赖的一次性服务通常是装好之后的收尾（比如建账号），跑完才算装好。被依赖的这时已经退出，马上返回。
        for container in plan.containers.iter().filter(|c| c.one_shot) {
            let finished = Target::Depend(DependCondition::ServiceCompletedSuccessfully);
            health::wait_for(&self.engine, &container.name, finished, deadline, langs).await?;
        }
        // 其余容器都必须健康或稳定运行。
        let mut healthy = Vec::new();
        for container in plan.containers.iter().filter(|c| !c.one_shot) {
            let readiness = health::wait_for(
                &self.engine,
                &container.name,
                Target::Ready,
                deadline,
                langs,
            )
            .await?;
            healthy.push(readiness == Readiness::Healthy);
        }
        Ok(Health::of(healthy))
    }

    /// 这套核心给插件建的容器、网络和卷：按标签找，别的核心（实例名不同）建的不算。
    pub(super) async fn own_resources(
        &self,
        id: &PluginId,
    ) -> Result<LabeledResources, InstallError> {
        let instance = &self.cfg.instance;
        let found = self
            .engine
            .labeled(LABEL_PLUGIN, id.as_str(), |labels| instance.owns(labels))
            .await?;
        Ok(found)
    }

    pub(super) async fn teardown(
        &self,
        id: &PluginId,
        purge: bool,
        langs: &Languages,
    ) -> Result<UninstallReport, InstallError> {
        let found = self.own_resources(id).await?;
        for container in &found.containers {
            self.engine.remove_container(container).await?;
        }
        // 容器都停了再吊销 Bot 的设备和模型网关凭据。
        let bots = self.deactivate_bots(id, purge).await?;
        self.revoke_models(id, langs).await?;
        self.remove_networks(&found.networks).await?;
        // 核心为只读根服务写的配置文件（configs_store）跟着插件删，不占数据目录的清理（purge）。
        let store_root = self.cfg.data_dir.join("core").join("configs");
        let store_dir = configs_store::plugin_dir(&store_root, &self.cfg.instance, &id.slug());
        configs_store::remove_tree(&store_root, &self.cfg.instance, &id.slug()).map_err(|e| {
            InstallError::Storage(l10n::text(
                langs,
                "plugin-install-teardown-delete-failed",
                &[
                    ("dir", store_dir.display().to_string().into()),
                    ("error", e.to_string().into()),
                ],
            ))
        })?;
        self.sources.remove(id);
        let mut report = UninstallReport {
            containers: found.containers.clone(),
            networks: found.networks.clone(),
            bots,
            ..Default::default()
        };
        if let Some(hydra) = &self.oidc {
            hydra.delete_client(id.as_str()).await?;
            report.oidc_client_removed = Some(!hydra.client_exists(id.as_str()).await?);
        }
        if purge {
            for volume in &found.volumes {
                self.engine.remove_volume(volume).await?;
            }
            report.volumes = found.volumes.clone();
            let dir = self.cfg.apps_dir.join(id.slug());
            match fs::remove_dir_all(&dir) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => {
                    return Err(InstallError::Storage(l10n::text(
                        langs,
                        "plugin-install-teardown-delete-failed",
                        &[
                            ("dir", dir.display().to_string().into()),
                            ("error", e.to_string().into()),
                        ],
                    )));
                }
            }
            self.store.secrets().remove(id)?;
            report.data_removed = true;
        }
        self.store.save_grants().clear(id)?;
        self.store.remove(id)?;
        let left = self.own_resources(id).await?;
        if !left.containers.is_empty()
            || !left.networks.is_empty()
            || (purge && !left.volumes.is_empty())
        {
            return Err(InstallError::Docker(l10n::text(
                langs,
                "plugin-install-teardown-leftovers",
                &[("left", format!("{left:?}").into())],
            )));
        }
        Ok(report)
    }
}

pub(super) fn client_spec(manifest: &Manifest, ctx: &InstallContext, secret: &str) -> ClientSpec {
    let webui = manifest.webui.as_ref();
    let base = ctx.app_url.as_deref().unwrap_or_default();
    let oidc = webui.and_then(|w| w.oidc.as_ref());
    ClientSpec {
        id: manifest.id.to_string(),
        secret: secret.to_owned(),
        name: manifest
            .title
            .get("en")
            .or_else(|| manifest.title.values().next())
            .cloned()
            .unwrap_or_else(|| manifest.id.to_string()),
        redirect_uris: oidc
            .map(|o| {
                o.redirect_paths
                    .iter()
                    .map(|p| format!("{base}{p}"))
                    .collect()
            })
            .unwrap_or_default(),
        post_logout_redirect_uris: webui
            .map(|webui| {
                // Hydra 按整串匹配，根地址有无末尾斜杠都要登记。
                let mut paths = vec!["", "/", webui.path.as_str()];
                if let Some(oidc) = oidc {
                    paths.extend(oidc.post_logout_paths.iter().map(String::as_str));
                }
                paths.sort_unstable();
                paths.dedup();
                paths
                    .into_iter()
                    .map(|path| format!("{base}{path}"))
                    .collect()
            })
            .unwrap_or_default(),
        token_endpoint_auth_method: oidc
            .map(|o| o.token_endpoint_auth_method.as_str())
            .unwrap_or("client_secret_basic"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owned_by(owner: Option<&str>) -> HashMap<String, String> {
        owner
            .map(|o| HashMap::from([(LABEL_PLUGIN.to_owned(), o.to_owned())]))
            .unwrap_or_default()
    }

    #[test]
    fn named_volumes_are_created_reused_or_refused_by_owner() {
        let plugin = PluginId::parse("app.foo-bar").unwrap();
        let langs = Languages::from_header(Some("en"));
        let action = |labels: Option<HashMap<String, String>>| {
            volume_action("wonton-app-foo-bar--data", labels.as_ref(), &plugin, &langs)
        };
        assert_eq!(action(None).unwrap(), VolumeAction::Create);
        assert_eq!(
            action(Some(owned_by(Some("app.foo-bar")))).unwrap(),
            VolumeAction::Reuse
        );
        // app.foo.bar 和 app.foo-bar 的 slug 相同，它卸载时保留的 data 卷同名：不能挂上别的插件的数据。
        let other = action(Some(owned_by(Some("app.foo.bar")))).unwrap_err();
        assert_eq!(other.kind(), "conflict", "{other}");
        let unlabeled = action(Some(owned_by(None))).unwrap_err();
        assert_eq!(unlabeled.kind(), "conflict", "{unlabeled}");
    }
}
