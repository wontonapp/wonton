//! 插件管理器：维护已安装插件的状态，对外提供预览、安装、卸载和网关路由查询。
//! 具体的 Docker 部署和回滚在 [`super::lifecycle`] 里。

use std::collections::{BTreeMap, BTreeSet};
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
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
use super::host_facts::NvidiaMemory;
use super::image_worker::ImageWorker;
use super::lan::Lan;
use super::languages::Languages;
use super::lifecycle::LifecycleConfig;
use super::listed::Listed;
use super::manifest::{LocalizedText, Manifest, ManifestExt, Permission, PluginId};
use super::mcp_server::{McpServer, ToolLimits};
#[cfg(test)]
use super::models::ModelsRoute;
use super::models::ProviderState;
use super::package::PluginPackage;
use super::sources::{Source, Sources};
use super::staged::StagedUploads;
use super::store::{InstalledRecord, Store};
use super::storefront::{StoreConfig, Storefront};
use super::upgrade::Upgrades;
use super::upstreams::Endpoint;
use crate::http::HttpClient;
use crate::l10n;
use crate::matrix::Matrix;

pub(super) mod busy;
mod clients;
pub(crate) use clients::ClientPlugin;
mod views;

pub use views::{ClientsRoute, HostedRoute, PluginView, Route, StorageRoute};

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
    /// Tuwunel 的服务器名，接上 Matrix 时一起定下；`matrix_rtc` 提供方要它（见 [`super::matrix_rtc`]）。
    pub(super) matrix_server_name: Option<String>,
    /// 给 `matrix_rtc` 别名签证书、给提供方写证书包的 CA；接了 Matrix 才有。
    pub(super) matrix_rtc_ca: Option<std::sync::Arc<super::matrix_rtc_ca::CoreCa>>,
    pub(super) mailpit: Option<Arc<crate::mailbox::Component>>,
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
    pub(super) bot_web_routes: RwLock<BTreeMap<String, super::bot_web::BotRoute>>,
    /// 交给 App 的图标。
    pub(super) gallery: Gallery,
    pub(crate) plugins: RwLock<BTreeMap<PluginId, Entry>>,
    pub(super) failed_installs: RwLock<BTreeMap<PluginId, super::install_failure::FailedInstall>>,
    pub(super) dependencies: super::dependencies::Dependencies,
    pub(crate) client_passwords: Option<Arc<crate::clients::passwords::PasswordStore>>,
    /// 访客链接（ADR-0090 第 5 节）；打不开时访客接口和入口上的链接都如实报不可用。
    pub(crate) client_guests: Option<Arc<crate::clients::guests::GuestStore>>,
    /// 跑应用的凭据命令；生产是 Docker，测试换成按队列给结果的假实现。
    pub(crate) credential_exec: RwLock<Arc<dyn crate::clients::credentials::CredentialExec>>,
    /// 每个插件一把异步锁：并发的启动/安装对账串行跑，第二个先看到第一个的结果。
    pub(crate) credential_locks:
        std::sync::Mutex<std::collections::BTreeMap<PluginId, Arc<tokio::sync::Mutex<()>>>>,
    /// 插件表每变一次加一：MCP 总线据此刷新工具列表（见 [`Self::changes`]）。
    changes: tokio::sync::watch::Sender<u64>,
    clients_gateway: std::sync::OnceLock<std::sync::Weak<crate::gateway::Gateway>>,
    pub(crate) activity: Arc<crate::bot_activity::BotActivity>,
    pub(crate) clients: Arc<crate::clients::state::State>,
    /// 随核心附带的插件（预装目录里的插件包），见 [`super::preinstall`]。
    pub(super) bundled: RwLock<BTreeSet<PluginId>>,
    /// 默认 Bot 插件，见 [`Self::default_bot_plugin`]。
    pub(super) default_bot_plugin: RwLock<Option<PluginId>>,
    /// 局域网端口开在哪里，见 [`super::lan`]。
    pub(super) lan: Lan,
    #[cfg(test)]
    pub(super) recovery_metadata_hold:
        std::sync::Mutex<Option<super::recovery_replace::MetadataHold>>,
    /// 出网插件走哪个代理，见 [`super::egress`]。
    pub(super) egress: Egress,
    /// 宿主主机名（`WONTON_HOST_NAME`）；只在取到时缓存，失败下次安装再取，见 [`super::host_check`]。
    pub(super) host_name: tokio::sync::OnceCell<String>,
    pub(super) registry_mirror: super::registry_mirror::RegistryMirror,
    /// 核心升级没结束时是真，这时不动插件（ADR-0025）。
    pub(super) upgrade_frozen: Option<tokio::sync::watch::Receiver<bool>>,
    /// 查到的新版本，见 [`super::upgrade`]。
    pub(super) upgrades: Upgrades,
    /// 拔掉和插回的设备，见 [`super::device::watch`]。
    pub(super) devices: super::device::watch::DeviceWatch,
    pub(super) oom: super::oom::OomRecords,
    pub(super) staged: StagedUploads,
    /// 写“每个人的语言”表时串行化：读改整份文件，两个人同时来不能互相盖掉。
    pub(super) people_writes: std::sync::Mutex<()>,
    pub(super) sharing:
        std::sync::OnceLock<(Arc<crate::friends::Friends>, Arc<crate::shares::Shares>)>,
    pub(super) person_members: std::sync::OnceLock<Arc<crate::members::Members>>,
    pub(super) person_roles: RwLock<Arc<crate::roles::Roles>>,
    /// Bot 已成立的联系人（ADR-0045），注入 `WONTON_MATRIX_ALLOWED_USERS` 时加在归属人后面。见
    /// [`super::contacts`]。
    pub(super) contact_source: std::sync::OnceLock<super::contacts::ContactSource>,
    /// 正在恢复已装插件的层数（核心刚起来、网关已经在接连接），见 [`Self::restoring_guard`]。
    restoring: std::sync::atomic::AtomicUsize,
    /// 各插件的 Bot 最后一次同步成功的时间，见 [`Self::note_bot_sync`]。
    bot_syncs: RwLock<BTreeMap<PluginId, tokio::time::Instant>>,
}

/// 恢复期间的标记，落地时清掉（出错、panic 也一样）。
pub(crate) struct Restoring<'a>(&'a PluginManager);

impl Drop for Restoring<'_> {
    fn drop(&mut self) {
        self.0.restoring.fetch_sub(1, Ordering::AcqRel);
    }
}

pub(super) use super::grant::{check_permissions, supported};

/// Bot 的长轮询最长挂 30 秒：超过这么久没有一次成功的同步，就是停了，不是正好在等消息。
const STALLED_SYNC_SECS: u64 = 120;

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
        let dependencies = super::dependencies::Dependencies::open(store.capabilities_path());
        let clients = Arc::new(crate::clients::state::State::new(&store, &cfg.public));
        let client_passwords = match crate::clients::passwords::PasswordStore::open(&cfg.data_dir) {
            Ok(passwords) => Some(Arc::new(passwords)),
            // 打开失败时客户端列表和开关都会报密码库不可用；留下原因，不然只能看到那句报错。
            Err(error) => {
                tracing::error!(%error, "客户端密码库打开失败");
                None
            }
        };
        let client_guests = match crate::clients::guests::GuestStore::open(&cfg.data_dir) {
            Ok(guests) => Some(Arc::new(guests)),
            Err(error) => {
                tracing::error!(%error, "访客链接库打开失败");
                None
            }
        };
        let credential_exec: Arc<dyn crate::clients::credentials::CredentialExec> = Arc::new(
            crate::clients::credentials::EngineExec::new(Arc::new(engine.clone())),
        );
        Self {
            client_passwords,
            client_guests,
            credential_exec: RwLock::new(credential_exec),
            credential_locks: Default::default(),
            engine,
            store,
            http,
            oidc,
            matrix: None,
            matrix_server_name: None,
            matrix_rtc_ca: None,
            mailpit: None,
            profiles: Default::default(),
            host: HostPaths::new(host_dev, None),
            nvidia_memory: NvidiaMemory::default(),
            cfg,
            sources: Sources::default(),
            github,
            storefront,
            core_proxy,
            bot_allocation: tokio::sync::Mutex::new(()),
            bot_web_routes: Default::default(),
            gallery: Gallery::new(images),
            plugins: RwLock::new(BTreeMap::new()),
            failed_installs: RwLock::new(BTreeMap::new()),
            dependencies,
            changes: tokio::sync::watch::Sender::new(0),
            clients,
            clients_gateway: std::sync::OnceLock::new(),
            activity: Arc::default(),
            bundled: RwLock::new(BTreeSet::new()),
            default_bot_plugin: RwLock::new(None),
            lan,
            #[cfg(test)]
            recovery_metadata_hold: Default::default(),
            egress: Egress::default(),
            host_name: tokio::sync::OnceCell::new(),
            registry_mirror: Default::default(),
            upgrade_frozen: None,
            oom: Default::default(),
            upgrades: Upgrades::default(),
            devices: Default::default(),
            staged: StagedUploads::default(),
            people_writes: std::sync::Mutex::new(()),
            sharing: std::sync::OnceLock::new(),
            person_members: std::sync::OnceLock::new(),
            person_roles: RwLock::new(Arc::new(crate::roles::Roles::initial())),
            contact_source: std::sync::OnceLock::new(),
            restoring: std::sync::atomic::AtomicUsize::new(0),
            bot_syncs: RwLock::new(BTreeMap::new()),
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
        self.refresh_clients();
        self.changes.send_modify(|generation| *generation += 1);
        self.activity.changed();
    }

    /// 入口快照里的规范对外地址跟着记录和域名后缀走：记录一变（`changed`）和挂上网关时各刷一次。
    fn refresh_clients(&self) {
        let suffix = self.clients_gateway().and_then(|gateway| {
            gateway
                .domain
                .as_ref()
                .and_then(|domain| domain.suffix().ok().flatten())
        });
        self.clients
            .refresh(&self.store, suffix.as_deref(), &self.cfg.public);
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
                listed
                    .manifest
                    .permissions
                    .iter()
                    .any(|p| p.risk() == Some(super::manifest::RiskLevel::Highest))
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

    /// 测试用：摘掉一个插件的来源，模拟它还没轮到恢复、来源没登记。
    #[cfg(test)]
    pub(crate) fn forget_source(&self, id: &PluginId) {
        self.sources.remove(id);
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

    /// 已装插件的清单里有没有这项权限（不看主人当时是否授予）。
    pub fn has_permission(&self, id: &PluginId, permission: &Permission) -> bool {
        self.plugins
            .read()
            .expect("插件表锁中毒")
            .get(id)
            .and_then(|entry| entry.listed.as_ref())
            .is_some_and(|listed| listed.manifest.permissions.contains(permission))
    }

    pub fn list(&self) -> Vec<PluginView> {
        let plugins = self.plugins.read().expect("插件表锁中毒");
        let failed = self.failed_installs.read().expect("安装结果锁中毒");
        plugins
            .iter()
            .map(|(id, entry)| self.view(id, entry))
            .chain(failed.iter().map(|(id, entry)| {
                let mut view = self.view(id, &entry.entry);
                view.url = None;
                view
            }))
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
        plugins
            .get(id)
            .map(|entry| entry.status.clone())
            .or_else(|| {
                self.failed_installs
                    .read()
                    .expect("安装结果锁中毒")
                    .get(id)
                    .map(|failed| failed.entry.status.clone())
            })
    }

    /// 插件的名字（多语言）。
    pub fn title(&self, id: &PluginId) -> Option<LocalizedText> {
        let plugins = self.plugins.read().expect("插件表锁中毒");
        plugins.get(id).map(|entry| entry.title.clone())
    }

    /// 一个 Bot 的显示名：清单 `bot.display_name`，没写时用插件标题。
    pub fn bot_name(&self, id: &PluginId, _bot: &str) -> Option<LocalizedText> {
        let plugins = self.plugins.read().expect("插件表锁中毒");
        let entry = plugins.get(id)?;
        entry
            .listed
            .as_ref()
            .and_then(|listed| listed.manifest.bot.as_ref())
            .and_then(|bot| bot.display_name.clone())
            .or_else(|| Some(entry.title.clone()))
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

    /// 随核心镜像附带、由核心自己维护的系统插件。从商店、来源或上传的同名 system 插件不算，
    /// 它们和普通插件一样能经接口升级、卸载（ADR-0108）。
    pub fn is_bundled_system(&self, id: &PluginId) -> bool {
        self.is_system(id) && self.bundled_system_record(id)
    }

    /// 记录或内存集合表明这个插件随核心带。界面层拿着 entry 时也用它，和四道闸门同一套判定。
    pub(super) fn bundled_system_record(&self, id: &PluginId) -> bool {
        if self.bundled.read().expect("插件表锁中毒").contains(id) {
            return true;
        }
        self.store
            .load(id)
            .ok()
            .flatten()
            .is_some_and(|record| record.bundled)
    }

    /// OIDC 客户端（ID 是插件的 `id`，Bot 的网页客户端按它的插件 ID 查）声明的管理员组和角色声明；
    /// 插件没登记时返回空值。
    pub fn oidc_identity_claims(
        &self,
        client_id: &str,
    ) -> (Vec<String>, Option<super::manifest::RoleClaim>) {
        let identity = match self.bot_web_oidc_identity(client_id) {
            Ok(identity) => identity,
            Err(error) => {
                tracing::warn!(%error, "Cannot read Bot OIDC identity");
                return (Vec::new(), None);
            }
        };
        let Ok(id) = PluginId::parse(
            identity
                .as_ref()
                .map_or(client_id, |bot| bot.plugin_id.as_str()),
        ) else {
            return (Vec::new(), None);
        };
        self.plugins
            .read()
            .expect("插件表锁中毒")
            .get(&id)
            .and_then(|entry| entry.listed.as_ref())
            .and_then(|listed| listed.manifest.webui.as_ref())
            .and_then(|webui| webui.oidc.as_ref())
            .map(|client| (client.admin_groups.clone(), client.role_claim.clone()))
            .unwrap_or_default()
    }

    /// 没装成的插件也如实列出来（标为异常、写明原因），主人看得到，不悄悄跳过。
    pub(super) fn mark_failed(&self, manifest: &Manifest, reason: String) {
        let entry = Entry {
            title: manifest.title.clone(),
            ..Entry::broken(&manifest.id, manifest.version.clone(), reason)
        };
        self.failed_installs
            .write()
            .expect("安装结果锁中毒")
            .remove(&manifest.id);
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
        let models = permissions.contains(&Permission::Models);
        self.sources.insert(&id, vec![subnet], permissions, models);
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

    /// 恢复期间为真：网关据此把 Bot 的请求答成可重试的 503，不答 403（上游适配器见 403 就停同步）。
    ///
    /// 数层数而不是一个位：两段恢复叠在一起时，先结束的那段不能把还在进行的那段说成恢复了。
    pub(crate) fn restoring(&self) -> bool {
        self.restoring.load(Ordering::Acquire) > 0
    }

    /// 从现在起 [`Self::restoring`] 为真，落地时清掉。
    pub(crate) fn restoring_guard(&self) -> Restoring<'_> {
        self.restoring.fetch_add(1, Ordering::AcqRel);
        Restoring(self)
    }

    /// 这个插件的 Bot 刚同步成功过一次。
    ///
    /// 记的是时间不是“健康”：Bot 走了长轮询，进程活着、健康检查说 connected 都可能只是状态文件的残影，
    /// 只有还经核心同步才算真的收得到消息（见 [`Self::stalled_bot_sync`]）。
    pub(crate) fn note_bot_sync(&self, id: &PluginId) {
        self.bot_syncs
            .write()
            .expect("Bot 同步时间的锁中毒")
            .insert(id.clone(), tokio::time::Instant::now());
    }

    /// 这个插件的 Bot 多久没同步了（秒），超过 [`STALLED_SYNC_SECS`] 才算停了。从没同步过的给 `None`：
    /// 不用长轮询的 Bot 插件不该被这条检查报异常。
    pub(crate) fn stalled_bot_sync(&self, id: &PluginId) -> Option<u64> {
        let syncs = self.bot_syncs.read().expect("Bot 同步时间的锁中毒");
        let age = syncs.get(id)?.elapsed().as_secs();
        (age > STALLED_SYNC_SECS).then_some(age)
    }

    /// 这个插件换了一份新实例（安装、启动、恢复重建、卸载）：上一次实例的同步时间不再说明问题。
    ///
    /// 不重置的话，停用超过两分钟的插件一启动就会被 [`Self::check_bot_sync`] 判成异常，Bot 还没来得及同步。
    pub(crate) fn forget_bot_sync(&self, id: &PluginId) {
        self.bot_syncs
            .write()
            .expect("Bot 同步时间的锁中毒")
            .remove(id);
    }

    /// 核心启动时：按记录恢复插件状态，把 OIDC 客户端同步到身份提供方。
    /// 单个插件恢复失败（包括它的记录损坏）只把它标为异常，不影响核心和其他插件启动。
    ///
    /// 这里是一个插件接一个插件串行的：漂移重建只等 `install_timeout`，不让一个起不来的慢插件把排在后面
    /// 的所有插件拖住几十分钟（见 [`super::health_gate::Gate`]）。超时保留续建账本，后台按间隔重试。
    pub async fn restore(&self) -> Result<(), InstallError> {
        let _restoring = self.restoring_guard();
        let langs = self.owner_languages();
        self.prune_orphaned_bot_web().await?;
        let files = self.store.load_all()?;
        let mut ordered = Vec::with_capacity(files.len());
        for file in files {
            let priority = match &file.record {
                Ok(record) => {
                    match PluginPackage::with_files(&record.source, record.files.clone(), &langs) {
                        Ok(package) => {
                            let provides = package.manifest.provides.as_ref();
                            let priority =
                                if super::egress::provider::provides_proxy(&package.manifest) {
                                    0
                                } else if provides.is_some_and(|provides| {
                                    provides.models.is_some() || !provides.capabilities.is_empty()
                                }) {
                                    1
                                } else {
                                    2
                                };
                            self.plugins.write().expect("插件表锁中毒").insert(
                                file.id.clone(),
                                Entry::new(
                                    Listed::of_record(&package, record),
                                    PluginStatus::Installing,
                                    None,
                                ),
                            );
                            self.forget_bot_sync(&file.id);
                            priority
                        }
                        Err(_) => 2,
                    }
                }
                Err(_) => 2,
            };
            ordered.push((priority, file));
        }
        ordered.sort_by_key(|(priority, _)| *priority);
        for (_, file) in ordered {
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
                .insert(file.id.clone(), entry);
            if let Ok(record) = &file.record
                && let Ok(package) =
                    PluginPackage::with_files(&record.source, record.files.clone(), &langs)
                && let Err(error) = self
                    .capabilities_installed(&package.manifest, &[], true)
                    .await
            {
                tracing::error!(plugin = %file.id, %error, "恢复能力连接失败");
            }
        }
        self.follow_core_proxy();
        self.changed();
        Ok(())
    }

    async fn restore_one(&self, record: &InstalledRecord, langs: &Languages) -> Entry {
        match record.package(langs) {
            Ok(package) if super::install_hosted::is_containerless(&package.manifest) => {
                self.restore_containerless(&package, record, langs)
            }
            Ok(package) => {
                // 不管核对结果如何都先登记来源：插件暂时不健康，也要能经核心重新连上 Matrix 恢复过来。
                let manifest = &package.manifest;
                let network = self.cfg.instance.network(&manifest.id.slug());
                if let Err(e) = self.register_source(manifest, &network).await {
                    tracing::warn!(plugin = %record.id, error = %e, "登记插件网络的地址段失败");
                }
                let listed = Listed::of_record(&package, record);
                match self.restore_runtime(&package, record).await {
                    Ok(entry) => entry,
                    Err(error) => Entry::containers_failed(listed, error.to_string()),
                }
            }
            Err(e) => Entry::broken(
                &record.id,
                record.version.clone(),
                unreadable_record(&e, langs),
            ),
        }
    }
}

/// 记录读不懂时给主人看的原因：比如核心升级后不再认记录里的字段。
pub(super) fn unreadable_record(error: &dyn std::fmt::Display, langs: &Languages) -> String {
    l10n::text(
        langs,
        "plugin-install-record-unreadable",
        &[("error", error.to_string().into())],
    )
}
