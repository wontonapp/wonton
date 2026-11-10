//! 插件管理器：维护已安装插件的状态，对外提供预览、安装、卸载和网关路由查询。
//! 具体的 Docker 部署和回滚在 [`super::lifecycle`] 里。

use std::collections::{BTreeMap, BTreeSet};
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use wonton_protocol::{InstallProgress, InstallStage};

use super::docker::Engine;
use super::egress::core_proxy::CoreProxy;
use super::egress::settings::Egress;
use super::gallery::Gallery;
use super::github::GitHub;
use super::gpu::HostPaths;
use super::health_gate::Gate;
use super::host_check;
use super::host_facts::NvidiaMemory;
use super::image_worker::ImageWorker;
use super::lan::Lan;
use super::lan::webui::switched;
use super::languages::Languages;
use super::lifecycle::LifecycleConfig;
use super::listed::Listed;
use super::manifest::{LocalizedText, Manifest, Permission, PluginId};
use super::mcp_server::{McpServer, ToolLimits};
#[cfg(test)]
use super::models::ModelsRoute;
use super::models::ProviderState;
use super::package::PluginPackage;
use super::render::dry_run;
use super::sources::{Source, Sources};
use super::staged::StagedUploads;
use super::store::{EnabledPermission, InstalledRecord, Store};
use super::storefront::{StoreConfig, Storefront};
use super::upgrade::Upgrades;
use super::upstreams::Endpoint;
use crate::http::HttpClient;
use crate::l10n;
use crate::matrix::Matrix;

pub(super) mod busy;
mod views;

pub use views::{PluginView, Route};

pub(super) use busy::{busy, detached};

pub(super) use super::entry::{Deployed, Entry};
pub use super::entry::{Health, PluginStatus};
pub use super::install_error::InstallError;
use crate::oidc::Hydra;

#[derive(Debug, Clone, Default, Serialize)]
pub struct UninstallReport {
    pub containers: Vec<String>,
    pub networks: Vec<String>,
    pub volumes: Vec<String>,
    pub data_removed: bool,
    /// 没有配置身份提供方时为空。
    pub oidc_client_removed: Option<bool>,
    /// 插件有 Bot 时：吊销了它们的设备，删除数据时还停用了账号。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub bots: Vec<BotReport>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BotReport {
    pub user_id: String,
    /// 吊销的设备数。
    pub devices_removed: usize,
    /// 账号已停用（不可恢复）。
    pub deactivated: bool,
}

pub struct PluginManager {
    pub(super) engine: Engine,
    pub(super) store: Store,
    pub(super) http: HttpClient,
    pub(super) oidc: Option<Hydra>,
    pub(super) matrix: Option<Arc<Matrix>>,
    pub(super) profiles: super::bot_profile::ProfileState,
    pub(super) cfg: LifecycleConfig,
    /// 核心看到的宿主路径，探测显卡用（见 [`super::gpu`]）。
    pub(super) host: HostPaths,
    pub(super) nvidia_memory: NvidiaMemory,
    pub(super) sources: Sources,
    /// 按来源取插件包。
    pub(super) github: GitHub,
    pub(super) core_proxy: CoreProxy,
    /// 官方索引和社区插件。
    pub(super) storefront: Storefront,
    /// 分配 Bot 用户名时持有：同时安装的两个插件不能分到同一个账号。
    pub(super) bot_allocation: tokio::sync::Mutex<()>,
    /// 交给 App 的图标。
    pub(super) gallery: Gallery,
    pub(super) plugins: RwLock<BTreeMap<PluginId, Entry>>,
    /// 插件表每变一次加一：MCP 总线据此刷新工具列表（见 [`Self::changes`]）。
    changes: tokio::sync::watch::Sender<u64>,
    pub(crate) activity: Arc<crate::bot_activity::BotActivity>,
    /// 随核心附带的插件（预装目录里的插件包），见 [`super::preinstall`]。
    pub(super) bundled: RwLock<BTreeSet<PluginId>>,
    /// 局域网端口开在哪里，见 [`super::lan`]。
    pub(super) lan: Lan,
    /// 出网插件走哪个代理，见 [`super::egress`]。
    pub(super) egress: Egress,
    /// 宿主主机名（`WONTON_HOST_NAME`）；只在取到时缓存，失败下次安装再取，见 [`super::host_check`]。
    pub(super) host_name: tokio::sync::OnceCell<String>,
    pub(super) registry_mirror: super::registry_mirror::RegistryMirror,
    /// 核心升级没结束时是真，这时不动插件（ADR-0025）。
    pub(super) upgrade_frozen: Option<tokio::sync::watch::Receiver<bool>>,
    /// 查到的新版本，见 [`super::upgrade`]。
    pub(super) upgrades: Upgrades,
    pub(super) oom: super::oom::OomRecords,
    pub(super) staged: StagedUploads,
    /// 写“每个人的语言”表时串行化：读改整份文件，两个人同时来不能互相盖掉。
    pub(super) people_writes: std::sync::Mutex<()>,
}

pub(super) use super::grant::{check_permissions, supported};

pub(crate) fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

impl PluginManager {
    /// 存数据的那块盘。查数据目录，不查 `apps_dir`：它要等装第一个插件时才建。
    pub fn storage_usage(&self) -> std::io::Result<wonton_protocol::StorageUsage> {
        let data_dir = &self.cfg.data_dir;
        let stats = rustix::fs::statvfs(data_dir)?;
        let block = stats.f_frsize.max(1);
        Ok(wonton_protocol::StorageUsage {
            total_bytes: stats.f_blocks.saturating_mul(block),
            free_bytes: stats.f_bavail.saturating_mul(block),
        })
    }

    pub fn new(
        engine: Engine,
        store: Store,
        http: HttpClient,
        oidc: Option<Hydra>,
        cfg: LifecycleConfig,
        images: impl Into<std::sync::Arc<ImageWorker>>,
    ) -> Self {
        // 核心直接跑在宿主上时 /dev 就是宿主的；在容器里要由 compose 挂载进来，见 [`Self::with_host`]。
        let host_dev = cfg.core_container.is_none().then(|| PathBuf::from("/dev"));
        // Docker 层的错误按主人最近一次请求的语言写：Engine 自己读不了 store，接上数据目录。
        let mut engine = engine;
        engine.set_languages(store.people_languages(), cfg.owner.clone());
        let lan = Lan::for_core(cfg.core_container.clone());
        let core_proxy = CoreProxy::default();
        let github = GitHub::new(&core_proxy);
        let storefront =
            Storefront::new(github.clone(), StoreConfig::default(), store.index_cache());
        Self {
            engine,
            store,
            http,
            oidc,
            matrix: None,
            profiles: Default::default(),
            host: HostPaths::new(host_dev),
            nvidia_memory: NvidiaMemory::default(),
            cfg,
            sources: Sources::default(),
            github,
            storefront,
            core_proxy,
            bot_allocation: tokio::sync::Mutex::new(()),
            gallery: Gallery::new(images),
            plugins: RwLock::new(BTreeMap::new()),
            changes: tokio::sync::watch::Sender::new(0),
            activity: Arc::default(),
            bundled: RwLock::new(BTreeSet::new()),
            lan,
            egress: Egress::default(),
            host_name: tokio::sync::OnceCell::new(),
            registry_mirror: Default::default(),
            upgrade_frozen: None,
            oom: Default::default(),
            upgrades: Upgrades::default(),
            staged: StagedUploads::default(),
            people_writes: std::sync::Mutex::new(()),
        }
    }

    /// 核心数据目录。
    pub fn data_dir(&self) -> PathBuf {
        self.cfg.data_dir.clone()
    }

    /// 插件装上、卸掉、状态变了时收到通知。
    pub fn changes(&self) -> tokio::sync::watch::Receiver<u64> {
        self.changes.subscribe()
    }

    pub(super) fn changed(&self) {
        self.changes.send_modify(|generation| *generation += 1);
        self.activity.changed();
    }

    /// 声明了 `mcp`、MCP 服务现在能用的插件：运行中，或者异常但 MCP 服务自己好着。
    pub fn mcp_servers(&self) -> Vec<McpServer> {
        let plugins = self.plugins.read().expect("插件表锁中毒");
        plugins
            .iter()
            .filter(|(_, entry)| {
                matches!(
                    entry.status,
                    PluginStatus::Running | PluginStatus::Degraded { .. }
                )
            })
            .filter_map(|(id, entry)| {
                let target = entry.mcp.clone()?;
                let mcp = entry.listed.as_ref()?.manifest.mcp.as_ref()?;
                if !entry.serving.contains(&mcp.service) {
                    return None;
                }
                Some(McpServer {
                    id: id.clone(),
                    title: entry.title.clone(),
                    url: target.url,
                    socket: target.socket,
                    headers: target.headers,
                    tool_limits: ToolLimits::of(Some(mcp)),
                    tools: mcp.tools.clone(),
                })
            })
            .collect()
    }

    /// 这个插件的最高级权限：它的工具每次都以 root 身份执行，不能放宽（ADR-0036）。
    pub fn requires_root_confirmation(&self, id: &PluginId) -> bool {
        let plugins = self.plugins.read().expect("插件表锁中毒");
        plugins
            .get(id)
            .and_then(|entry| entry.listed.as_ref())
            .is_some_and(|listed| {
                listed.manifest.permissions.iter().any(|p| {
                    super::privileges::risk(p) == Some(super::privileges::CapabilityRisk::Highest)
                })
            })
    }

    /// 插件装上的清单要了 `owner.identity`：它能看见人的身份，读家人名单和别人的语言都靠它。
    pub fn gets_owner_identity(&self, id: &PluginId) -> bool {
        self.plugins
            .read()
            .expect("插件表锁中毒")
            .get(id)
            .and_then(|entry| entry.listed.as_ref())
            .is_some_and(|listed| listed.manifest.gets_owner_identity())
    }

    /// 来源地址落在哪个已安装插件的网络里（见 [`super::sources`]）。
    pub fn source(&self, ip: IpAddr) -> Option<Source> {
        self.sources.find(ip)
    }

    /// 模型网关现在的状态：没装、装了没在运行（或者网关服务不可用）、可用。
    pub(super) fn models_provider(&self) -> ProviderState {
        let plugins = self.plugins.read().expect("插件表锁中毒");
        let Some((id, entry)) = plugins.iter().find(|(_, e)| e.provides_models) else {
            return ProviderState::Absent;
        };
        match (&entry.status, &entry.models) {
            (PluginStatus::Running | PluginStatus::Degraded { .. }, Some(route))
                if entry.serving.contains(&route.spec.service) =>
            {
                ProviderState::Ready(Box::new(route.clone()))
            }
            _ => ProviderState::NotReady(id.clone()),
        }
    }

    pub fn list(&self) -> Vec<PluginView> {
        let plugins = self.plugins.read().expect("插件表锁中毒");
        plugins
            .iter()
            .map(|(id, entry)| self.view(id, entry))
            .collect()
    }

    /// 处理插件包里的图标和截图（校验插件包时用，`plugin check` 同一份代码）。
    pub fn images(&self) -> &Arc<ImageWorker> {
        self.gallery.images()
    }

    /// 插件记录和密钥库（卡片里要用密钥库判断哪一项已经填过）。
    pub(crate) fn store(&self) -> &Store {
        &self.store
    }

    pub(crate) fn is_installed(&self, id: &PluginId) -> bool {
        self.plugins.read().expect("插件表锁中毒").contains_key(id)
    }

    pub(super) fn status_of(&self, id: &PluginId) -> Option<PluginStatus> {
        let plugins = self.plugins.read().expect("插件表锁中毒");
        plugins.get(id).map(|entry| entry.status.clone())
    }

    /// 插件的名字（多语言）。
    pub fn title(&self, id: &PluginId) -> Option<LocalizedText> {
        let plugins = self.plugins.read().expect("插件表锁中毒");
        plugins.get(id).map(|entry| entry.title.clone())
    }

    /// 插件有没有网页：通知里带 `path` 的前提。
    pub fn has_webui(&self, id: &PluginId) -> bool {
        self.plugins
            .read()
            .expect("插件表锁中毒")
            .get(id)
            .is_some_and(|entry| entry.has_webui)
    }

    /// 是安装器预装的系统插件。
    pub fn is_system(&self, id: &PluginId) -> bool {
        self.plugins
            .read()
            .expect("插件表锁中毒")
            .get(id)
            .is_some_and(|entry| entry.system)
    }

    /// OIDC 客户端（ID 是插件的 `id`）在 `owner` 之外要发的组名（清单 `oidc.owner_groups`）。
    /// 客户端没登记、没声明或已卸载时为空，只有 `owner`。
    pub fn oidc_owner_groups(&self, client_id: &str) -> Vec<String> {
        let Ok(id) = PluginId::parse(client_id) else {
            return Vec::new();
        };
        self.plugins
            .read()
            .expect("插件表锁中毒")
            .get(&id)
            .and_then(|entry| entry.listed.as_ref())
            .and_then(|listed| listed.manifest.webui.as_ref())
            .and_then(|webui| webui.oidc.as_ref())
            .map(|client| client.owner_groups.clone())
            .unwrap_or_default()
    }

    /// 没装成的插件也如实列出来（标为异常、写明原因），主人看得到，不悄悄跳过。
    pub(super) fn mark_failed(&self, manifest: &Manifest, reason: String) {
        let entry = Entry {
            title: manifest.title.clone(),
            ..Entry::broken(&manifest.id, manifest.version.clone(), reason)
        };
        self.plugins
            .write()
            .expect("插件表锁中毒")
            .insert(manifest.id.clone(), entry);
        self.changed();
    }

    /// 插件现在是不是 `reason` 这个异常。
    pub(super) fn is_failed_with(&self, id: &PluginId, reason: &str) -> bool {
        let plugins = self.plugins.read().expect("插件表锁中毒");
        plugins
            .get(id)
            .is_some_and(|entry| entry.failed_with(reason))
    }

    /// 收回 [`Self::mark_failed`] 标的异常，好重新安装：插件还是 `reason` 这个异常才收回，返回收回了没有。
    pub(super) fn take_failed(&self, id: &PluginId, reason: &str) -> bool {
        let mut plugins = self.plugins.write().expect("插件表锁中毒");
        let failed = plugins
            .get(id)
            .is_some_and(|entry| entry.failed_with(reason));
        if failed {
            plugins.remove(id);
        }
        failed
    }

    /// 测试用：把登记过的插件当成运行中的模型网关，供网关测试指向本地的假上游。
    #[cfg(test)]
    pub(crate) fn serve_models_for_test(&self, route: ModelsRoute) {
        let mut plugins = self.plugins.write().expect("插件表锁中毒");
        let entry = plugins.get_mut(&route.id).expect("先 register_for_test");
        entry.provides_models = true;
        entry.models = Some(route);
    }

    /// 测试用：登记一个插件网络的地址段，供网关测试按来源地址认出插件。
    #[cfg(test)]
    pub(crate) fn register_source_for_test(
        &self,
        id: &str,
        subnet: &str,
        permissions: Vec<Permission>,
    ) {
        let id = PluginId::parse(id).unwrap();
        let subnet = super::sources::Subnet::parse(subnet).unwrap();
        self.sources.insert(&id, vec![subnet], permissions);
    }

    pub(super) fn set(
        &self,
        id: &PluginId,
        status: PluginStatus,
        upstream: Option<Option<Endpoint>>,
    ) {
        let mut plugins = self.plugins.write().expect("插件表锁中毒");
        if let Some(entry) = plugins.get_mut(id) {
            entry.status = status;
            entry.install_progress = None;
            entry.health = None;
            entry.checked_at = now_secs();
            entry.follows_containers = false;
            if let Some(upstream) = upstream {
                entry.upstream = upstream;
            }
        }
        drop(plugins);
        self.changed();
    }

    /// 报一步阶段，不带百分比。
    pub(super) fn stage(&self, id: &PluginId, stage: InstallStage) {
        self.report_progress(
            id,
            InstallProgress {
                stage,
                percent: None,
            },
        );
    }

    /// 安装或升级报到的阶段。阶段变了、或者百分比又前进了 5 才算换了一格，免得拉镜像时每一块都叫醒一轮列表。
    pub(super) fn report_progress(&self, id: &PluginId, progress: InstallProgress) {
        let stepped = {
            let mut plugins = self.plugins.write().expect("插件表锁中毒");
            let Some(entry) = plugins.get_mut(id) else {
                return;
            };
            let stepped = match &entry.install_progress {
                None => true,
                Some(current) => {
                    current.stage != progress.stage
                        || match (current.percent, progress.percent) {
                            (None, None) => false,
                            (Some(before), Some(now)) => {
                                now == 100 || now.saturating_sub(before) >= 5
                            }
                            _ => true,
                        }
                }
            };
            if stepped {
                entry.install_progress = Some(progress);
            }
            stepped
        };
        if stepped {
            self.changed();
        }
    }

    /// 核心启动时：按记录恢复插件状态，把 OIDC 客户端同步到身份提供方。
    /// 单个插件恢复失败（包括它的记录损坏）只把它标为异常，不影响核心和其他插件启动。
    ///
    /// 这里是一个插件接一个插件串行的：漂移重建只等 `install_timeout`，不让一个起不来的慢插件把排在后面
    /// 的所有插件拖住几十分钟（见 [`Gate`]）。超时的插件容器留着，健康以后由容器监视捡回来。
    pub async fn restore(&self) -> Result<(), InstallError> {
        let langs = self.owner_languages();
        let files = self.store.load_all()?;
        // 先登记代理提供方：恢复其他插件时就可能需要解析它的地址。
        for file in &files {
            if let Ok(record) = &file.record
                && let Ok(package) =
                    PluginPackage::with_files(&record.source, record.files.clone(), &langs)
                && super::egress::provider::provides_proxy(&package.manifest)
            {
                let listed = Listed::of_record(&package, record);
                self.plugins.write().expect("插件表锁中毒").insert(
                    file.id.clone(),
                    Entry::new(listed, PluginStatus::Installing, None),
                );
            }
        }
        for file in files {
            let entry = match &file.record {
                Ok(record) => self.restore_one(record, &langs).await,
                Err(e) => Entry::broken(&file.id, String::new(), unreadable_record(&e, &langs)),
            };
            match &entry.status {
                PluginStatus::Running => tracing::info!(plugin = %file.id, "已恢复插件"),
                PluginStatus::Stopped => tracing::info!(plugin = %file.id, "插件已停止"),
                status => tracing::warn!(plugin = %file.id, ?status, "插件状态异常"),
            }
            self.plugins
                .write()
                .expect("插件表锁中毒")
                .insert(file.id, entry);
        }
        self.follow_core_proxy();
        self.changed();
        Ok(())
    }

    async fn restore_one(&self, record: &InstalledRecord, langs: &Languages) -> Entry {
        match PluginPackage::with_files(&record.source, record.files.clone(), langs) {
            Ok(mut package) => {
                package.origin = record.origin.clone();
                // 不管核对结果如何都先登记来源：插件暂时不健康，也要能经核心重新连上 Matrix 恢复过来。
                let manifest = &package.manifest;
                let network = self.cfg.instance.network(&manifest.id.slug());
                if let Err(e) = self.register_source(manifest, &network).await {
                    tracing::warn!(plugin = %record.id, error = %e, "登记插件网络的地址段失败");
                }
                let listed = Listed::of_record(&package, record);
                let lan_ports = listed.open_ports(record.lan_webui);
                match self.reconcile(&package, record, Gate::InstallTimeout).await {
                    // 有的服务没就绪时路由照样回来了：照常让局域网检测核一遍地址（见下面）。
                    Ok(Some(deployed)) => {
                        if deployed.problem.is_some() && !lan_ports.is_empty() {
                            self.lan.mark_degraded(&manifest.id);
                        }
                        Entry::running(listed, deployed)
                    }
                    Ok(None) => Entry::stopped(listed),
                    Err(e) => {
                        // 宿主重启后 DHCP 换了地址时，容器发布不到旧地址上、起不来：由局域网检测按新地址重建。
                        if !lan_ports.is_empty() {
                            self.lan.mark_degraded(&manifest.id);
                        }
                        Entry::containers_failed(listed, e.to_string())
                    }
                }
            }
            Err(e) => Entry::broken(
                &record.id,
                record.version.clone(),
                unreadable_record(&e, langs),
            ),
        }
    }

    /// 检查并部署，API 安装和预装（见 [`super::preinstall`]）共用。
    pub(super) async fn install_checked(
        &self,
        package: PluginPackage,
        granted: &[Permission],
        enabled: &[EnabledPermission],
        lan_webui: bool,
        langs: &Languages,
    ) -> Result<PluginView, InstallError> {
        super::install_entry::check_runtime(&package.manifest, langs)?;
        self.check_not_upgrading(langs)?;
        dry_run(&package, langs)?;
        self.check_icon(&package, langs).await?;
        let manifest = package.manifest.clone();
        check_permissions(&manifest, granted, enabled, langs)?;
        super::capabilities::unsupported_dependencies(&manifest, langs)?;
        self.check_requirements(&manifest, langs).await?;
        self.check_egress(&manifest, langs).await?;
        let gpu = self.gpu_grant(&manifest, granted, langs).await?;
        // 插件没有这个开关时不记：以后的版本有了开关也是默认关。
        let lan_webui = lan_webui && switched(&package).is_some();
        let lan = self.lan_grant(&package, lan_webui, langs).await?;
        let listed = Listed::new(&package, now_secs(), lan_webui, false, Default::default());
        self.reserve(&listed, langs)?;
        tracing::info!(plugin = %manifest.id, gpu = ?gpu, lan = ?lan, "开始安装插件");
        match self
            .deploy(&package, granted, enabled, gpu, lan, langs)
            .await
        {
            Ok(deployed) => {
                tracing::info!(plugin = %manifest.id, health = ?deployed.health, "插件安装完成");
                let listed = Listed::new(
                    &package,
                    now_secs(),
                    lan_webui,
                    deployed.firewalled,
                    Default::default(),
                );
                let view = {
                    let mut plugins = self.plugins.write().expect("插件表锁中毒");
                    let entry = Entry::running(listed, deployed);
                    let view = self.view(&manifest.id, &entry);
                    plugins.insert(manifest.id.clone(), entry);
                    view
                };
                self.changed();
                self.probe_fresh_consumer(&manifest).await;
                // 装的是模型网关：给没接上的使用方签发凭据、按新变量重建（见 super::models）。
                if manifest
                    .provides
                    .as_ref()
                    .is_some_and(|p| p.models.is_some())
                {
                    for (plugin, error) in self.reconcile_models().await {
                        tracing::error!(%plugin, %error, "模型网关装好了，接上使用方失败");
                    }
                }
                Ok(view)
            }
            Err(e) => {
                tracing::warn!(plugin = %manifest.id, error = %e, "插件安装失败，已回滚");
                self.plugins
                    .write()
                    .expect("插件表锁中毒")
                    .remove(&manifest.id);
                self.changed();
                Err(e)
            }
        }
    }

    /// 占位，防止同一插件并发安装，也防止两个插件映射到同一个子域名。
    fn reserve(&self, listed: &Arc<Listed>, langs: &Languages) -> Result<(), InstallError> {
        let manifest = &listed.manifest;
        let mut plugins = self.plugins.write().expect("插件表锁中毒");
        if let Some(existing) = plugins.get(&manifest.id) {
            return Err(
                busy(&manifest.id, &existing.status, langs).unwrap_or_else(|| {
                    InstallError::Conflict(l10n::text(
                        langs,
                        "plugin-install-conflict-already-installed",
                        &[("id", manifest.id.to_string().into())],
                    ))
                }),
            );
        }
        self.check_proxy_provider(manifest, &plugins, langs)?;
        let slug = manifest.id.slug();
        if let Some(other) = plugins.keys().find(|id| id.slug() == slug) {
            return Err(InstallError::Conflict(l10n::text(
                langs,
                "plugin-install-conflict-slug",
                &[
                    ("id", manifest.id.to_string().into()),
                    ("other", other.to_string().into()),
                ],
            )));
        }
        plugins.insert(
            manifest.id.clone(),
            Entry::new(listed.clone(), PluginStatus::Installing, None),
        );
        drop(plugins);
        self.changed();
        Ok(())
    }

    pub(super) async fn check_requirements(
        &self,
        manifest: &Manifest,
        langs: &Languages,
    ) -> Result<(), InstallError> {
        {
            let plugins = self.plugins.read().expect("插件表锁中毒");
            self.check_proxy_provider(manifest, &plugins, langs)?;
        }
        let provides_models = manifest
            .provides
            .as_ref()
            .is_some_and(|p| p.models.is_some());
        let existing = match self.models_provider() {
            ProviderState::Absent => None,
            ProviderState::NotReady(id) => Some(id),
            ProviderState::Ready(route) => Some(route.id),
        };
        if provides_models && let Some(id) = existing.filter(|id| *id != manifest.id) {
            return Err(InstallError::Conflict(l10n::text(
                langs,
                "plugin-install-conflict-models-gateway",
                &[("id", id.to_string().into())],
            )));
        }
        let info = self.engine.host_info().await?;
        let facts = self.facts_from(Ok(info), langs).await;
        host_check::refusal(manifest, &facts, langs).map_or(Ok(()), Err)
    }

    /// 同 [`Self::uninstall`]，但在独立的任务里执行（见 [`detached`]）：网关和 MCP 总线都经这里卸载。
    pub async fn uninstall_detached(
        self: &Arc<Self>,
        id: PluginId,
        purge: bool,
        langs: Languages,
    ) -> Result<UninstallReport, InstallError> {
        let this = Arc::clone(self);
        detached(async move { this.uninstall(&id, purge, &langs).await }).await
    }

    /// 卸载插件。`purge` 时一并删除命名卷和应用数据目录。
    pub async fn uninstall(
        &self,
        id: &PluginId,
        purge: bool,
        langs: &Languages,
    ) -> Result<UninstallReport, InstallError> {
        self.check_not_upgrading(langs)?;
        let _changing = self.egress.changing.lock().await;
        if self.egress_config()?.source == wonton_protocol::EgressProxySource::Plugin
            && self.egress_provider(langs)?.id == *id
        {
            return Err(InstallError::Conflict(l10n::text(
                langs,
                "plugin-install-conflict-egress-proxy-in-use",
                &[],
            )));
        }
        {
            let mut plugins = self.plugins.write().expect("插件表锁中毒");
            let entry = plugins
                .get_mut(id)
                .ok_or_else(|| InstallError::not_installed(langs, id))?;
            if let Some(e) = busy(id, &entry.status, langs) {
                return Err(e);
            }
            if self.lan.is_rebuilding(id) {
                return Err(InstallError::Conflict(l10n::text(
                    langs,
                    "plugin-install-conflict-lan-rebuilding",
                    &[("id", id.to_string().into())],
                )));
            }
            entry.status = PluginStatus::Uninstalling;
            entry.checked_at = now_secs();
        }
        drop(_changing);
        self.changed();
        tracing::info!(plugin = %id, purge, "开始卸载插件");
        match self.teardown(id, purge, langs).await {
            Ok(report) => {
                self.clear_oom(id);
                self.lan.clear_degraded(id);
                self.plugins.write().expect("插件表锁中毒").remove(id);
                self.tool_policy().forget_plugin(id.as_str());
                self.changed();
                tracing::info!(plugin = %id, "插件已卸载");
                Ok(report)
            }
            Err(e) => {
                let reason = l10n::text(
                    langs,
                    "plugin-install-uninstall-unfinished",
                    &[("error", e.to_string().into())],
                );
                self.set(id, PluginStatus::Degraded { reason }, Some(None));
                Err(e)
            }
        }
    }
}

/// 记录读不懂时给主人看的原因：比如核心升级后不再认记录里的字段。
fn unreadable_record(error: &dyn std::fmt::Display, langs: &Languages) -> String {
    l10n::text(
        langs,
        "plugin-install-record-unreadable",
        &[("error", error.to_string().into())],
    )
}
