//! 插件管理器：维护已安装插件的状态，对外提供预览、安装、卸载和网关路由查询。
//! 具体的 Docker 部署和回滚在 [`super::lifecycle`] 里。

use std::collections::{BTreeMap, BTreeSet};
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

use super::docker::Engine;
use super::egress::core_proxy::CoreProxy;
use super::egress::settings::Egress;
use super::gallery::Gallery;
use super::github::GitHub;
use super::gpu::HostPaths;
use super::image_worker::ImageWorker;
use super::lan::Lan;
use super::lan::webui::switched;
use super::lifecycle::LifecycleConfig;
use super::manifest::{Arch, LocalizedText, Manifest, Permission, PluginId, PluginKind};
use super::mcp_server::{McpServer, ToolLimits};
use super::models::{ModelsRoute, ProviderState};
use super::package::PluginPackage;
use super::proxy_auth::ProxyAuth;
use super::render::dry_run;
use super::sources::{Source, Sources};
use super::store::{InstalledRecord, Store};
use super::storefront::{StoreConfig, Storefront};
use super::upgrade::Upgrades;
use super::views::Listed;
use crate::http::HttpClient;
use crate::matrix::Matrix;

pub use super::install_error::InstallError;
use crate::oidc::Hydra;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum PluginStatus {
    Installing,
    Running,
    /// 已安装但状态异常（容器缺失或停止、卸载中途失败等），原因如实给出。
    Degraded {
        reason: String,
    },
    Uninstalling,
    /// 正在换成新版本（见 [`super::upgrade`]）。
    Upgrading,
}

/// 运行中的插件健不健康，是最近一次检查的结果（见 [`super::watch`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Health {
    /// 每个长期运行的服务都有健康检查，而且都通过了。
    Healthy,
    /// 有长期运行的服务没有健康检查（或者检查时还没通过）：只知道它在运行，不能说它健康。
    Unknown,
}

impl Health {
    /// 所有长期运行的服务都健康才算健康。
    pub fn of(healthy: impl IntoIterator<Item = bool>) -> Self {
        if healthy.into_iter().all(|h| h) {
            Self::Healthy
        } else {
            Self::Unknown
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct PluginView {
    pub id: PluginId,
    pub version: String,
    pub title: LocalizedText,
    pub status: PluginStatus,
    /// 只有运行中的插件才有。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub health: Option<Health>,
    pub url: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct UninstallReport {
    pub containers: Vec<String>,
    pub networks: Vec<String>,
    pub volumes: Vec<String>,
    pub data_removed: bool,
    /// 没有配置身份提供方时为空。
    pub oidc_client_removed: Option<bool>,
    /// 插件有 Bot 时：吊销了它的设备，删除数据时还停用了账号。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bot: Option<BotReport>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BotReport {
    pub user_id: String,
    /// 吊销的设备数。
    pub devices_removed: usize,
    /// 账号已停用（不可恢复）。
    pub deactivated: bool,
}

/// 网关查询用的路由信息。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Route {
    pub id: PluginId,
    pub status: PluginStatus,
    /// `host:port`，核心可直接访问的插件网页地址。
    pub upstream: Option<String>,
    /// `sso: proxy` 时网关代加的凭据。
    pub proxy: Option<Arc<ProxyAuth>>,
}

pub(super) struct Entry {
    pub(super) version: String,
    pub(super) title: LocalizedText,
    /// `kind: system`：由安装器预装。
    pub(super) system: bool,
    pub(super) has_webui: bool,
    pub(super) status: PluginStatus,
    pub(super) health: Option<Health>,
    /// 列表和详情页显示的信息；记录读不出来的插件为空。
    pub(super) listed: Option<Arc<Listed>>,
    upstream: Option<String>,
    /// 声明了 `provides.models`（不论状态）。
    pub(super) provides_models: bool,
    /// 运行中的模型网关的转发目标。
    models: Option<ModelsRoute>,
    /// `sso: proxy` 时网关代加的凭据。
    proxy: Option<Arc<ProxyAuth>>,
    /// 运行中时 MCP 服务的地址。
    mcp: Option<String>,
    /// 这个异常是容器决定的，容器好了就恢复（见 [`super::watch`]）。
    pub(super) follows_containers: bool,
}

/// 部署或核对完成的插件：核心访问它网页的地址（是模型网关时还有网关接口的地址），和检查时的健康状况。
pub(super) struct Deployed {
    pub upstream: Option<String>,
    pub health: Health,
    pub models: Option<ModelsRoute>,
    pub proxy: Option<Arc<ProxyAuth>>,
    pub mcp: Option<String>,
}

impl Entry {
    pub(super) fn new(listed: Arc<Listed>, status: PluginStatus, upstream: Option<String>) -> Self {
        let manifest = &listed.manifest;
        Self {
            version: manifest.version.clone(),
            title: manifest.title.clone(),
            system: manifest.kind == PluginKind::System,
            has_webui: manifest.webui.is_some(),
            status,
            health: None,
            upstream,
            provides_models: manifest
                .provides
                .as_ref()
                .is_some_and(|p| p.models.is_some()),
            models: None,
            proxy: None,
            mcp: None,
            listed: Some(listed),
            follows_containers: false,
        }
    }

    pub(super) fn running(listed: Arc<Listed>, deployed: Deployed) -> Self {
        Self {
            health: Some(deployed.health),
            models: deployed.models,
            proxy: deployed.proxy,
            mcp: deployed.mcp,
            ..Self::new(listed, PluginStatus::Running, deployed.upstream)
        }
    }

    /// 核对没过的插件：之后跟着容器走（见 [`super::watch`]），容器好了再核对一次就恢复。
    pub(super) fn containers_failed(listed: Arc<Listed>, reason: String) -> Self {
        Self {
            follows_containers: true,
            ..Self::new(listed, PluginStatus::Degraded { reason }, None)
        }
    }

    /// 记录或其中的插件包读不出来：只知道 ID（版本可能也不知道），如实标为异常，主人可以卸载它。
    fn broken(id: &PluginId, version: String, reason: String) -> Self {
        Self {
            version,
            title: ["zh", "en"]
                .map(|lang| (lang.to_owned(), id.to_string()))
                .into(),
            system: false,
            has_webui: false,
            status: PluginStatus::Degraded { reason },
            health: None,
            upstream: None,
            provides_models: false,
            models: None,
            proxy: None,
            mcp: None,
            listed: None,
            follows_containers: false,
        }
    }

    pub(super) fn failed_with(&self, reason: &str) -> bool {
        matches!(&self.status, PluginStatus::Degraded { reason: r } if r == reason)
    }
}

pub struct PluginManager {
    pub(super) engine: Engine,
    pub(super) store: Store,
    pub(super) http: HttpClient,
    pub(super) oidc: Option<Hydra>,
    pub(super) matrix: Option<Arc<Matrix>>,
    pub(super) cfg: LifecycleConfig,
    /// 核心看到的宿主路径，探测显卡用（见 [`super::gpu`]）。
    pub(super) host: HostPaths,
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
    /// 随核心附带的插件（预装目录里的插件包），见 [`super::preinstall`]。
    pub(super) bundled: RwLock<BTreeSet<PluginId>>,
    /// 局域网端口开在哪里，见 [`super::lan`]。
    pub(super) lan: Lan,
    /// 出网插件走哪个代理，见 [`super::egress`]。
    pub(super) egress: Egress,
    /// 核心升级没结束时是真，这时不动插件（ADR-0025）。
    pub(super) upgrade_frozen: Option<tokio::sync::watch::Receiver<bool>>,
    /// 查到的新版本，见 [`super::upgrade`]。
    pub(super) upgrades: Upgrades,
}

/// 核心已经兑现的权限。其余权限在规范里定义了，但核心还不能兑现，安装时如实拒绝。
pub(super) fn supported(permission: &Permission) -> bool {
    matches!(
        permission,
        Permission::StorageShare { .. }
            | Permission::NetworkEgress
            | Permission::MatrixBot
            | Permission::Models
            | Permission::Tools
            | Permission::OwnerIdentity
            | Permission::DevicesGpu
            | Permission::NetworkLan
    )
}

pub(crate) fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

impl PluginManager {
    /// 存数据的那块盘。查数据目录，不查 `apps_dir`：它要等装第一个插件时才建。
    pub fn storage_usage(&self) -> std::io::Result<wonton_protocol::StorageUsage> {
        let data_dir = self.cfg.apps_dir.parent().unwrap_or(&self.cfg.apps_dir);
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
            host: HostPaths::new(host_dev),
            cfg,
            sources: Sources::default(),
            github,
            storefront,
            core_proxy,
            bot_allocation: tokio::sync::Mutex::new(()),
            gallery: Gallery::new(images),
            plugins: RwLock::new(BTreeMap::new()),
            changes: tokio::sync::watch::Sender::new(0),
            bundled: RwLock::new(BTreeSet::new()),
            lan,
            egress: Egress::default(),
            upgrade_frozen: None,
            upgrades: Upgrades::default(),
        }
    }

    /// 插件装上、卸掉、状态变了时收到通知。
    pub fn changes(&self) -> tokio::sync::watch::Receiver<u64> {
        self.changes.subscribe()
    }

    pub(super) fn changed(&self) {
        self.changes.send_modify(|generation| *generation += 1);
    }

    /// 在运行、声明了 `mcp` 的插件。
    pub fn mcp_servers(&self) -> Vec<McpServer> {
        let plugins = self.plugins.read().expect("插件表锁中毒");
        plugins
            .iter()
            .filter(|(_, entry)| entry.status == PluginStatus::Running)
            .filter_map(|(id, entry)| {
                let url = entry.mcp.clone()?;
                let title = entry.title.clone();
                let listed = entry.listed.as_ref();
                let tool_limits = ToolLimits::of(listed.and_then(|l| l.manifest.mcp.as_ref()));
                Some(McpServer {
                    id: id.clone(),
                    title,
                    url,
                    tool_limits,
                })
            })
            .collect()
    }

    /// 来源地址落在哪个已安装插件的网络里（见 [`super::sources`]）。
    pub fn source(&self, ip: IpAddr) -> Option<Source> {
        self.sources.find(ip)
    }

    /// 模型网关现在的状态：没装、装了没在运行、可用。
    pub(super) fn models_provider(&self) -> ProviderState {
        let plugins = self.plugins.read().expect("插件表锁中毒");
        let Some((id, entry)) = plugins.iter().find(|(_, e)| e.provides_models) else {
            return ProviderState::Absent;
        };
        match (&entry.status, &entry.models) {
            (PluginStatus::Running, Some(route)) => ProviderState::Ready(Box::new(route.clone())),
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

    pub fn route(&self, slug: &str) -> Option<Route> {
        let plugins = self.plugins.read().expect("插件表锁中毒");
        plugins
            .iter()
            .find(|(id, _)| id.slug() == slug)
            .map(|(id, entry)| Route {
                id: id.clone(),
                status: entry.status.clone(),
                upstream: entry.upstream.clone(),
                proxy: entry.proxy.clone(),
            })
    }

    fn view(&self, id: &PluginId, entry: &Entry) -> PluginView {
        PluginView {
            id: id.clone(),
            version: entry.version.clone(),
            title: entry.title.clone(),
            status: entry.status.clone(),
            health: entry.health,
            url: entry.has_webui.then(|| self.web_url(id)),
        }
    }

    /// 插件网页的地址。
    pub(super) fn web_url(&self, id: &PluginId) -> String {
        self.cfg
            .public
            .origin(&self.cfg.public.plugin_host(&id.slug()))
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

    /// 是安装器预装的系统插件。
    pub fn is_system(&self, id: &PluginId) -> bool {
        self.plugins
            .read()
            .expect("插件表锁中毒")
            .get(id)
            .is_some_and(|entry| entry.system)
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

    /// 测试用：不经 Docker 直接登记一个插件，供网关测试指向本地的假上游。
    #[cfg(test)]
    pub(crate) fn register_for_test(
        &self,
        manifest: Manifest,
        status: PluginStatus,
        upstream: Option<String>,
    ) {
        let id = manifest.id.clone();
        let entry = Entry::new(Listed::of_manifest(manifest), status, upstream);
        self.plugins
            .write()
            .expect("插件表锁中毒")
            .insert(id, entry);
    }

    /// 测试用：给登记过的插件设上 MCP 服务的地址。
    #[cfg(test)]
    pub(crate) fn mcp_for_test(&self, id: &PluginId, url: String) {
        let mut plugins = self.plugins.write().expect("插件表锁中毒");
        plugins.get_mut(id).expect("先 register_for_test").mcp = Some(url);
        drop(plugins);
        self.changed();
    }

    /// 测试用：给登记过的插件设上网关代登录的凭据。
    #[cfg(test)]
    pub(crate) fn proxy_for_test(&self, id: &PluginId, auth: ProxyAuth) {
        let mut plugins = self.plugins.write().expect("插件表锁中毒");
        plugins.get_mut(id).expect("先 register_for_test").proxy = Some(Arc::new(auth));
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
        upstream: Option<Option<String>>,
    ) {
        let mut plugins = self.plugins.write().expect("插件表锁中毒");
        if let Some(entry) = plugins.get_mut(id) {
            entry.status = status;
            entry.health = None;
            entry.follows_containers = false;
            if let Some(upstream) = upstream {
                entry.upstream = upstream;
            }
        }
        drop(plugins);
        self.changed();
    }

    /// 核心启动时：按记录恢复插件状态，把 OIDC 客户端同步到身份提供方。
    /// 单个插件恢复失败（包括它的记录损坏）只把它标为异常，不影响核心和其他插件启动。
    pub async fn restore(&self) -> Result<(), InstallError> {
        for file in self.store.load_all()? {
            let entry = match &file.record {
                Ok(record) => self.restore_one(record).await,
                Err(e) => Entry::broken(&file.id, String::new(), e.to_string()),
            };
            match &entry.status {
                PluginStatus::Running => tracing::info!(plugin = %file.id, "已恢复插件"),
                status => tracing::warn!(plugin = %file.id, ?status, "插件状态异常"),
            }
            self.plugins
                .write()
                .expect("插件表锁中毒")
                .insert(file.id, entry);
        }
        self.changed();
        Ok(())
    }

    async fn restore_one(&self, record: &InstalledRecord) -> Entry {
        match PluginPackage::with_files(&record.source, record.files.clone()) {
            Ok(mut package) => {
                package.origin = record.origin.clone();
                // 不管核对结果如何都先登记来源：插件暂时不健康，也要能经核心重新连上 Matrix 恢复过来。
                let manifest = &package.manifest;
                let network = self.cfg.instance.network(&manifest.id.slug());
                let migrated = self.migrate_network(&network, manifest).await;
                if let Err(e) = self.register_source(manifest, &network).await {
                    tracing::warn!(plugin = %record.id, error = %e, "登记插件网络的地址段失败");
                }
                let listed = Listed::new(&package, record.installed_at, record.lan_webui);
                match self.reconcile_migrated(migrated, &package, record).await {
                    Ok(deployed) => Entry::running(listed, deployed),
                    Err(e) => {
                        // 宿主重启后 DHCP 换了地址时，容器发布不到旧地址上、起不来：由局域网检测按新地址重建。
                        if !listed.open_ports(record.lan_webui).is_empty() {
                            self.lan.mark_degraded(&manifest.id);
                        }
                        Entry::containers_failed(listed, e.to_string())
                    }
                }
            }
            Err(e) => Entry::broken(
                &record.id,
                record.version.clone(),
                format!("记录里的插件包无法解析：{e}"),
            ),
        }
    }

    /// 检查并部署，API 安装和预装（见 [`super::preinstall`]）共用。
    pub(super) async fn install_checked(
        &self,
        package: PluginPackage,
        granted: &[Permission],
        lan_webui: bool,
    ) -> Result<PluginView, InstallError> {
        self.check_not_upgrading()?;
        dry_run(&package)?;
        self.check_icon(&package).await?;
        let manifest = package.manifest.clone();
        check_permissions(&manifest, granted)?;
        self.check_requirements(&manifest).await?;
        self.check_egress(&manifest).await?;
        let gpu = self.gpu_grant(&manifest).await?;
        // 插件没有这个开关时不记：以后的版本有了开关也是默认关。
        let lan_webui = lan_webui && switched(&package).is_some();
        let lan = self.lan_grant(&package, lan_webui).await?;
        let listed = Listed::new(&package, now_secs(), lan_webui);
        self.reserve(&listed)?;
        tracing::info!(plugin = %manifest.id, gpu = ?gpu, lan = ?lan, "开始安装插件");
        match self.deploy(&package, granted, gpu, lan).await {
            Ok(deployed) => {
                tracing::info!(plugin = %manifest.id, health = ?deployed.health, "插件安装完成");
                let mut plugins = self.plugins.write().expect("插件表锁中毒");
                let entry = Entry::running(listed, deployed);
                let view = self.view(&manifest.id, &entry);
                plugins.insert(manifest.id.clone(), entry);
                drop(plugins);
                self.changed();
                Ok(view)
            }
            Err(e) => {
                tracing::warn!(plugin = %manifest.id, error = %e, "插件安装失败，已回滚");
                self.plugins
                    .write()
                    .expect("插件表锁中毒")
                    .remove(&manifest.id);
                Err(e)
            }
        }
    }

    /// 占位，防止同一插件并发安装，也防止两个插件映射到同一个子域名。
    fn reserve(&self, listed: &Arc<Listed>) -> Result<(), InstallError> {
        let manifest = &listed.manifest;
        let mut plugins = self.plugins.write().expect("插件表锁中毒");
        if let Some(existing) = plugins.get(&manifest.id) {
            return Err(busy(&manifest.id, &existing.status).unwrap_or_else(|| {
                InstallError::Conflict(format!("应用 {} 已经装了", manifest.id))
            }));
        }
        let slug = manifest.id.slug();
        if let Some(other) = plugins.keys().find(|id| id.slug() == slug) {
            return Err(InstallError::Conflict(format!(
                "应用 {} 与已安装的 {other} 使用同一个网页地址",
                manifest.id
            )));
        }
        plugins.insert(
            manifest.id.clone(),
            Entry::new(listed.clone(), PluginStatus::Installing, None),
        );
        Ok(())
    }

    pub(super) async fn check_requirements(&self, manifest: &Manifest) -> Result<(), InstallError> {
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
            return Err(InstallError::Conflict(format!(
                "已经装了模型网关 {id}：同一时间只能有一个"
            )));
        }
        let arch = self.engine.host_arch().await?;
        let wanted = manifest.requirements.as_ref().and_then(|r| r.arch.as_ref());
        match (wanted, Arch::from_docker(&arch)) {
            (Some(list), Some(host)) if !list.contains(&host) => Err(InstallError::Unsupported(
                format!("应用不支持本机架构 {arch}"),
            )),
            (Some(_), None) => Err(InstallError::Unsupported(format!(
                "无法识别本机架构 {arch}"
            ))),
            _ => Ok(()),
        }
    }

    /// 同 [`Self::uninstall`]，但在独立的任务里执行（见 [`detached`]）：网关和 MCP 总线都经这里卸载。
    pub async fn uninstall_detached(
        self: &Arc<Self>,
        id: PluginId,
        purge: bool,
    ) -> Result<UninstallReport, InstallError> {
        let this = Arc::clone(self);
        detached(async move { this.uninstall(&id, purge).await }).await
    }

    /// 卸载插件。`purge` 时一并删除命名卷和应用数据目录。
    pub async fn uninstall(
        &self,
        id: &PluginId,
        purge: bool,
    ) -> Result<UninstallReport, InstallError> {
        self.check_not_upgrading()?;
        {
            let mut plugins = self.plugins.write().expect("插件表锁中毒");
            let entry = plugins
                .get_mut(id)
                .ok_or_else(|| InstallError::NotInstalled(id.to_string()))?;
            if let Some(e) = busy(id, &entry.status) {
                return Err(e);
            }
            if self.lan.is_rebuilding(id) {
                let reason = format!("插件 {id} 正在按新的局域网地址重建");
                return Err(InstallError::Conflict(reason));
            }
            entry.status = PluginStatus::Uninstalling;
        }
        tracing::info!(plugin = %id, purge, "开始卸载插件");
        match self.teardown(id, purge).await {
            Ok(report) => {
                self.lan.clear_degraded(id);
                self.plugins.write().expect("插件表锁中毒").remove(id);
                self.changed();
                tracing::info!(plugin = %id, "插件已卸载");
                Ok(report)
            }
            Err(e) => {
                let reason = format!("卸载未完成：{e}");
                self.set(id, PluginStatus::Degraded { reason }, Some(None));
                Err(e)
            }
        }
    }
}

/// 插件正在安装、卸载或升级时，对它的新操作如实回答“正在进行”。
pub(super) fn busy(id: &PluginId, status: &PluginStatus) -> Option<InstallError> {
    let doing = match status {
        PluginStatus::Installing => "安装",
        PluginStatus::Uninstalling => "卸载",
        PluginStatus::Upgrading => "升级",
        PluginStatus::Running | PluginStatus::Degraded { .. } => return None,
    };
    Some(InstallError::Conflict(format!("应用 {id} 正在{doing}")))
}

/// 在独立的任务里执行，调用方只等结果：调用方被丢弃（请求断开：手机切网、隧道断开）时操作照样做完，不会停在
/// 半路，留下一直“安装中”“卸载中”的插件和装了一半的资源。任务不会被取消（只有运行时关闭时才会，那时调用方
/// 也已经没了），所以出错只可能是任务 panic，原样传给调用方。
pub(super) async fn detached<T: Send + 'static>(op: impl Future<Output = T> + Send + 'static) -> T {
    match tokio::spawn(op).await {
        Ok(out) => out,
        Err(e) => std::panic::resume_unwind(e.into_panic()),
    }
}

pub(super) fn check_permissions(
    manifest: &Manifest,
    granted: &[Permission],
) -> Result<(), InstallError> {
    if let Some(extra) = granted.iter().find(|p| !manifest.permissions.contains(p)) {
        return Err(InstallError::Invalid(format!(
            "授予了清单没有声明的权限 {extra}"
        )));
    }
    let missing: Vec<String> = manifest
        .permissions
        .iter()
        .filter(|p| !granted.contains(p))
        .map(ToString::to_string)
        .collect();
    if !missing.is_empty() {
        return Err(InstallError::PermissionNotGranted(missing));
    }
    if let Some(p) = manifest.permissions.iter().find(|p| !supported(p)) {
        return Err(InstallError::Unsupported(format!("核心还不支持权限 {p}")));
    }
    Ok(())
}
