//! 插件管理器：维护已安装插件的状态，对外提供预览、安装、卸载和网关路由查询。
//! 具体的 Docker 部署和回滚在 [`super::lifecycle`] 里。

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::Ordering;
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

use super::docker::Engine;
use super::egress::core_proxy::CoreProxy;
use super::egress::settings::Egress;
use super::gallery::Gallery;
use super::github::GitHub;
use super::gpu::HostPaths;
use super::host_facts::NvidiaMemory;
use super::lan::Lan;
use super::lifecycle::LifecycleConfig;
use super::manifest::{LocalizedText, PluginId};
use super::sources::Sources;
use super::staged::StagedUploads;
use super::store::{InstalledRecord, Store};
use super::storefront::Storefront;
use super::upgrade::Upgrades;
use crate::http::HttpClient;
use crate::matrix::Matrix;

mod build;
pub(super) mod busy;
mod clients;
pub(crate) use clients::ClientPlugin;
mod query;
mod restore;
pub(in crate::plugin) use restore::unreadable_record;
mod sync;
mod update;
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

pub(crate) fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}
