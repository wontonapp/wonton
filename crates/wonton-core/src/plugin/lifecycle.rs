//! 部署、恢复与卸载：把部署计划交给 Docker，失败时按相反顺序回滚。

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tokio::time::Instant;

use super::compose::DependCondition;
use super::configs;
use super::core_address;
use super::docker::{LabeledResources, NetworkReach};
use super::gpu::GpuGrant;
use super::health::{self, Readiness, Target};
use super::hostdir::{self, Owner};
use super::lan::LanGrant;
use super::leftovers::name_taken;
use super::manager::{Deployed, Health, InstallError, PluginManager, UninstallReport};
use super::manifest::{Manifest, Permission, PluginId, Sso};
use super::names::{Instance, LABEL_PLUGIN};
use super::package::PluginPackage;
use super::proxy_auth::{self, ProxyAuth};
use super::render::{Deployment, InstallContext, OidcVars, OwnerVars, render};
use super::secrets::{self, Secrets};
use super::store::InstalledRecord;
use super::upstreams::{core_aliases, uses_sso};
use crate::config::PublicUrls;
use crate::matrix::ProvisionedBot;
use crate::oidc::{ClientSpec, Hydra};
use crate::secret::random_token;

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
    pub(super) bot: Option<(PluginId, ProvisionedBot)>,
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
) -> Result<VolumeAction, InstallError> {
    let Some(labels) = labels else {
        return Ok(VolumeAction::Create);
    };
    if labels.get(LABEL_PLUGIN).map(String::as_str) == Some(plugin.as_str()) {
        Ok(VolumeAction::Reuse)
    } else {
        Err(name_taken("卷", volume, labels))
    }
}

impl PluginManager {
    pub(super) fn hydra(&self) -> Result<&Hydra, InstallError> {
        self.oidc.as_ref().ok_or_else(|| {
            InstallError::OidcUnavailable(
                "核心没有配置身份提供方（WONTON_OIDC_ADMIN_URL / WONTON_OIDC_PUBLIC_URL）".into(),
            )
        })
    }

    /// 部署插件并写下安装记录，返回核心访问插件网页的地址。任一步失败（包括写记录）都回滚本次创建的
    /// 资源：没有记录的容器、网络和 OIDC 客户端，核心重启后就没人管了。`gpu` 是交给 `gpu.services` 的显卡，`lan`
    /// 是局域网端口开在哪里。
    pub(super) async fn deploy(
        &self,
        package: &PluginPackage,
        granted: &[Permission],
        gpu: Option<GpuGrant>,
        lan: Option<LanGrant>,
    ) -> Result<Deployed, InstallError> {
        let mut undo = Undo::default();
        match self
            .deploy_inner(package, granted, (gpu, lan), &mut undo)
            .await
        {
            Ok(deployed) => Ok(deployed),
            Err(error) => {
                let failures = self.rollback(undo).await;
                if failures.is_empty() {
                    Err(error)
                } else {
                    Err(error
                        .with_note(&format!("回滚未完成，可能有残留：{}", failures.join("；"))))
                }
            }
        }
    }

    async fn deploy_inner(
        &self,
        package: &PluginPackage,
        granted: &[Permission],
        (gpu, lan): (Option<GpuGrant>, Option<LanGrant>),
        undo: &mut Undo,
    ) -> Result<Deployed, InstallError> {
        let manifest = &package.manifest;
        self.clear_leftovers(&manifest.id).await?;
        let secret = uses_sso(manifest).then(random_token);
        let mut secrets = self.prepare_secrets(manifest, undo)?;
        let generated =
            secrets::generated_values(manifest, &secrets).map_err(InstallError::Storage)?;
        let proxy = ProxyAuth::resolve(manifest, &generated).map_err(InstallError::Storage)?;
        let mut ctx = self
            .install_context(manifest, secret.as_deref(), generated)
            .await?;
        ctx.gpu = gpu;
        ctx.lan = lan;
        self.check_network_names(&ctx).await?;
        if let Some(bot) = self.provision_bot(package).await? {
            ctx.matrix = Some(self.matrix_vars(&bot));
            undo.bot = Some((manifest.id.clone(), bot));
        }
        if let Some(models) = self.provision_models(package).await? {
            ctx.models = Some(models);
            undo.models = Some(manifest.id.clone());
        }
        let mut plan = render(package, &ctx)?;
        self.prepare_images(&mut plan).await?;
        self.prepare_dirs(&ctx, &plan.dirs, undo)?;
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
        let mut plan = render(package, &ctx)?;
        super::registry_mirror::use_pulled_images(&mut plan, pulled);
        for volume in &plan.volumes {
            let labels = self.engine.volume_labels(volume).await?;
            if volume_action(volume, labels.as_ref(), &manifest.id)? == VolumeAction::Create {
                self.engine.create_volume(volume, &plan.labels).await?;
                undo.volumes.push(volume.clone());
            }
        }
        if let Some(secret) = &secret {
            self.hydra()?
                .upsert_client(&client_spec(manifest, &ctx, secret))
                .await?;
            undo.oidc_client = Some(manifest.id.to_string());
        }
        self.rewrite_image_healthchecks(&ctx, &mut plan).await?;
        let deadline = Instant::now() + self.cfg.install_timeout;
        let health = self.start_containers(&plan, undo, deadline).await?;
        if secrets::collect(&self.engine, &self.cfg.instance, manifest, &mut secrets).await? {
            self.store.secrets().save(&manifest.id, &secrets)?;
        }
        let upstream = self.webui_upstream(manifest, &plan.network).await?;
        let models = self.models_upstream(manifest, &plan.network).await?;
        let mcp = self.mcp_upstream(manifest, &plan.network).await?;
        if let (Some(upstream), Some(webui)) = (&upstream, &manifest.webui) {
            health::wait_webui(&self.http, upstream, &webui.path, deadline).await?;
        }
        let firewalled = self
            .host_webui_firewalled(manifest, &plan.relays, &plan.network)
            .await;
        if let Some((_, bot)) = &undo.bot {
            undo.handed_over = self.hand_over_home_room(bot).await?;
        }
        self.store.save(&InstalledRecord {
            id: manifest.id.clone(),
            version: manifest.version.clone(),
            source: package.source.clone(),
            files: package.files().clone(),
            granted_permissions: granted.to_vec(),
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
        })?;
        Ok(Deployed {
            upstream,
            health,
            models,
            proxy: proxy.map(Arc::new),
            mcp,
            firewalled,
        })
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

    pub(super) async fn install_context(
        &self,
        manifest: &Manifest,
        secret: Option<&str>,
        generated: BTreeMap<String, String>,
    ) -> Result<InstallContext, InstallError> {
        if self.cfg.profile.email().is_none() && proxy_auth::uses_email(manifest) {
            return Err(InstallError::OwnerProfileIncomplete);
        }
        let public = &self.cfg.public;
        let slug = manifest.id.slug();
        let app_url = manifest
            .webui
            .as_ref()
            .map(|_| public.origin(&public.plugin_host(&slug)));
        let oidc = match secret {
            None => None,
            Some(secret) => {
                let discovery = self.hydra()?.discovery().await?;
                Some(OidcVars {
                    issuer: discovery.issuer,
                    client_id: manifest.id.to_string(),
                    client_secret: secret.to_owned(),
                    auth_url: discovery.authorization_endpoint,
                    token_url: discovery.token_endpoint,
                    userinfo_url: discovery.userinfo_endpoint,
                })
            }
        };
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
                let dir = self.cfg.shares_dir.display();
                InstallError::Storage(format!("创建共享文件夹目录 {dir} 失败：{e}"))
            })?;
        }
        for path in shares.values() {
            hostdir::ensure_owned_dir(path, self.owner())
                .map_err(|e| InstallError::Storage(format!("准备共享文件夹失败：{e}")))?;
        }
        Ok(InstallContext {
            app_data: self.cfg.apps_dir.join(&slug),
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
            models: None,
            mcp_url: manifest.permissions.contains(&Permission::Tools).then(|| {
                let port = self.cfg.gateway_port;
                format!("http://{}:{port}/mcp", crate::config::PLUGIN_MCP_HOST)
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
            cgroup_parent: self.cfg.plugin_slice.clone(),
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
    ) -> Result<(), InstallError> {
        if dirs.is_empty() {
            return Ok(());
        }
        let storage =
            |e: &dyn std::fmt::Display| InstallError::Storage(format!("准备应用数据目录失败：{e}"));
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
        undo: &mut Undo,
        deadline: Instant,
    ) -> Result<Health, InstallError> {
        for container in &plan.containers {
            for (dependency, condition) in &container.depends_on {
                health::wait_for(
                    &self.engine,
                    dependency,
                    Target::Depend(*condition),
                    deadline,
                )
                .await?;
            }
            self.engine
                .create_container(&container.name, container.body.clone())
                .await?;
            undo.containers.push(container.name.clone());
            if !container.files.is_empty() {
                let archive = configs::archive(&container.files).map_err(|e| {
                    InstallError::Invalid(format!("打包 {} 的配置文件失败：{e}", container.name))
                })?;
                self.engine
                    .upload_archive(&container.name, "/", archive)
                    .await?;
            }
            self.engine.start_container(&container.name).await?;
        }
        // 没人依赖的一次性服务通常是装好之后的收尾（比如建账号），跑完才算装好。被依赖的这时已经退出，马上返回。
        for container in plan.containers.iter().filter(|c| c.one_shot) {
            let finished = Target::Depend(DependCondition::ServiceCompletedSuccessfully);
            health::wait_for(&self.engine, &container.name, finished, deadline).await?;
        }
        // 其余容器都必须健康或稳定运行。
        let mut healthy = Vec::new();
        for container in plan.containers.iter().filter(|c| !c.one_shot) {
            let readiness =
                health::wait_for(&self.engine, &container.name, Target::Ready, deadline).await?;
            healthy.push(readiness == Readiness::Healthy);
        }
        Ok(Health::of(healthy))
    }

    pub(super) async fn rollback(&self, undo: Undo) -> Vec<String> {
        let mut failures = Vec::new();
        let mut note = |what: String, result: Result<(), String>| {
            if let Err(e) = result {
                tracing::error!(%what, error = %e, "回滚失败");
                failures.push(format!("{what}：{e}"));
            }
        };
        for name in undo.containers.iter().rev() {
            note(
                format!("容器 {name}"),
                self.engine
                    .remove_container(name)
                    .await
                    .map_err(|e| e.to_string()),
            );
        }
        if let Some((id, bot)) = &undo.bot {
            // 收回主房间要用 Bot 的令牌，所以在吊销设备之前。
            if undo.handed_over {
                note(
                    format!("交给你的主房间 {}", bot.account.home_room),
                    self.withdraw_home_room(bot).await,
                );
            }
            note(
                format!("Bot {} 的设备", bot.account.user_id),
                self.revoke_bot_device(id, bot).await,
            );
        }
        if let Some(id) = &undo.models {
            note(
                format!("插件 {id} 的模型网关凭据"),
                self.revoke_models(id).await.map_err(|e| e.to_string()),
            );
        }
        if let Some(id) = &undo.source {
            self.sources.remove(id);
        }
        if let (Some(id), Some(hydra)) = (&undo.oidc_client, &self.oidc) {
            note(
                format!("OIDC 客户端 {id}"),
                hydra.delete_client(id).await.map_err(|e| e.to_string()),
            );
        }
        for volume in &undo.volumes {
            note(
                format!("卷 {volume}"),
                self.engine
                    .remove_volume(volume)
                    .await
                    .map_err(|e| e.to_string()),
            );
        }
        if let Some(network) = &undo.lan_network {
            let result = self.engine.remove_network(network).await;
            note(format!("网络 {network}"), result.map_err(|e| e.to_string()));
        }
        if let Some(network) = &undo.network {
            if let (true, Some(core)) = (undo.core_attached, &self.cfg.core_container) {
                let result = self.engine.disconnect_network(network, core).await;
                note(
                    format!("核心退出网络 {network}"),
                    result.map_err(|e| e.to_string()),
                );
            }
            let result = self.detach_relay(network).await;
            note(
                format!("出口容器退出网络 {network}"),
                result.map_err(|e| e.to_string()),
            );
            note(
                format!("网络 {network}"),
                self.engine
                    .remove_network(network)
                    .await
                    .map_err(|e| e.to_string()),
            );
        }
        if let Some(dir) = &undo.app_dir_created {
            note(
                format!("目录 {}", dir.display()),
                fs::remove_dir_all(dir).map_err(|e| e.to_string()),
            );
        }
        if let Some((id, before)) = &undo.secrets_before {
            let store = self.store.secrets();
            let restored = match before {
                Some(secrets) => store.save(id, secrets),
                None => store.remove(id),
            };
            note(
                format!("插件 {id} 的密钥"),
                restored.map_err(|e| e.to_string()),
            );
        }
        failures
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
    ) -> Result<UninstallReport, InstallError> {
        let found = self.own_resources(id).await?;
        for container in &found.containers {
            self.engine.remove_container(container).await?;
        }
        // 容器都停了再吊销 Bot 的设备和模型网关凭据。
        let bot = self.remove_bot(id, purge).await?;
        self.revoke_models(id).await?;
        self.remove_networks(&found.networks).await?;
        self.sources.remove(id);
        let mut report = UninstallReport {
            containers: found.containers.clone(),
            networks: found.networks.clone(),
            bot,
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
                    return Err(InstallError::Storage(format!(
                        "删除 {} 失败：{e}",
                        dir.display()
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
            return Err(InstallError::Docker(format!("卸载后仍有残留：{left:?}")));
        }
        Ok(report)
    }
}

pub(super) fn client_spec(manifest: &Manifest, ctx: &InstallContext, secret: &str) -> ClientSpec {
    let webui = manifest.webui.as_ref();
    let base = ctx.app_url.clone().unwrap_or_default();
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
        let action = |labels: Option<HashMap<String, String>>| {
            volume_action("wonton-app-foo-bar--data", labels.as_ref(), &plugin)
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
