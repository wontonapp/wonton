//! 插件页面的状态（ADR-0017 的“客户端核心”）：已安装的插件、商店、取列表及启动的问题，以及安装、升级、卸载的进展。文字由
//! 家里的服务器按主人的语言挑好，这里原样放进 ViewModel。

pub(crate) mod api;
mod cache;
pub(crate) mod cards;
mod core_upgrade;
mod egress;
mod me;
mod media;
mod now;
mod polling;
mod registry_mirror;
mod sharing;
mod start;
mod support;
mod takeover;
mod tasks;
mod tool_policy;

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::Duration;

use reqwest::Client;
use tokio::runtime::Handle;
use tokio::sync::{Notify, watch};
use wonton_protocol::{
    AddSourceRequest, AddedSource, AskEveryUpdate, CoreError, CoreUpgradeViewModel,
    EgressViewModel, HomeStatus, InstallPreview, InstallSource, InstalledPlugins, LanWebuiUpdate,
    PermissionUpdate, PluginDetail, PluginFailure, PluginImage, PluginProblem, PluginTaskState,
    PluginsViewModel, ProxySettings, StorageUsage, StoreConfirmation, StoreItem, StoreListing,
    TabRef,
};

use crate::home::HomeLink;
use crate::port_map::PortMapper;
use crate::scope::reuse;
use api::{CoreApi, INSTALL_TIMEOUT, READ_TIMEOUT, SOURCE_TIMEOUT, proxied_client, to_core_error};
pub(crate) use support::lock;
use support::*;

/// 有插件在安装、升级、卸载时，隔这么久重新取一次已安装列表。
const POLL_INTERVAL: Duration = Duration::from_secs(3);

/// `PluginsViewModel.problem` 是哪个请求留下的。
#[derive(Clone, PartialEq, Eq)]
enum Listing {
    Installed,
    Store,
    Start(String),
}

pub(crate) struct PluginsLink {
    runtime: Handle,
    home: Arc<HomeLink>,
    external: bool,
    view: watch::Sender<PluginsViewModel>,
    languages: Mutex<Vec<String>>,
    /// 本地代理的凭据每次启动都换；同一个代理上复用连接。
    client: Mutex<Option<(ProxySettings, Client)>>,
    images_dir: PathBuf,
    /// 正在进行的列表请求，大于 0 时 `refreshing` 为真。
    in_flight: AtomicUsize,
    /// 主人打开过商店：隧道重新连上时也重新取它。
    store_wanted: AtomicBool,
    polling: AtomicBool,
    foreground: AtomicBool,
    active_page: Mutex<Option<TabRef>>,
    poll_changed: Notify,
    /// `view.problem` 来自哪个请求。
    problem_of: Mutex<Option<Listing>>,
    /// 已安装列表存在本机的位置。
    installed_cache: PathBuf,
    /// 每换一台服务器加一：之前发出的请求和任务，结果不再算数。
    generation: AtomicU64,
    /// 每次取或改出网代理的设置加一：只有最后发起的那次能写 `view.egress`。
    egress_epoch: AtomicU64,
    upgrade: Mutex<core_upgrade::UpgradeLocal>,
    /// 服务器报过的版本：404 分“服务器太旧”时用，取不到就不判旧。
    server_version: api::ServerVersion,
    /// 只有测试会关：升级状态的定时轮询会让请求在测试不控制的时刻 settle，盖不住观察者要看的先后顺序。
    upgrade_polling_paused: AtomicBool,
    registry: Mutex<registry_mirror::RegistryLocal>,
    takeover: Mutex<takeover::Slot>,
    starts: Mutex<HashMap<String, u64>>,
    data_dir: PathBuf,
    /// 发起安装、升级时的预览，按插件 ID 记着：失败后重试要来源，聊天列表的安装行要 `has_bot`。做完被主人清掉时删。
    previews: Mutex<HashMap<String, InstallPreview>>,
    /// “全部更新”里排着队等主人拿主意的确认，一次一条放进 `view.confirmation`；换服务器时随作用域作废。
    confirmations: Mutex<VecDeque<StoreConfirmation>>,
    /// 插件端口在本机的同号映射（ADR-0072），结果写进 `view.port_mappings`。
    ports: Arc<PortMapper>,
    /// 网页用摄像头/麦克风的记忆，结果写进 `view.media_grants`。
    media: media::MediaGrants,
}

impl PluginsLink {
    pub(crate) fn new(
        runtime: Handle,
        home: Arc<HomeLink>,
        data_dir: &Path,
        view: Option<watch::Sender<PluginsViewModel>>,
        external: bool,
    ) -> Arc<Self> {
        let installed_cache = data_dir.join("installed-plugins.json");
        let installed = home
            .view()
            .server_id
            .and_then(|server| cache::load(&installed_cache, &server));
        let view = reuse(
            view,
            PluginsViewModel {
                installed,
                actions: if external {
                    Vec::new()
                } else {
                    vec![
                        wonton_protocol::AppsAction::Store,
                        wonton_protocol::AppsAction::AddSource,
                    ]
                },
                ..PluginsViewModel::default()
            },
        );
        let ports = PortMapper::new(&runtime, home.clone(), data_dir, view.clone());
        let media = media::MediaGrants::new(data_dir, view.clone());
        let link = Arc::new(Self {
            runtime: runtime.clone(),
            home,
            external,
            view,
            languages: Mutex::new(Vec::new()),
            client: Mutex::new(None),
            images_dir: data_dir.join("plugin-images"),
            in_flight: AtomicUsize::new(0),
            store_wanted: AtomicBool::new(false),
            polling: AtomicBool::new(false),
            foreground: AtomicBool::new(true),
            active_page: Mutex::new(None),
            poll_changed: Notify::new(),
            problem_of: Mutex::new(None),
            installed_cache,
            generation: AtomicU64::new(0),
            egress_epoch: AtomicU64::new(0),
            upgrade: Mutex::new(core_upgrade::UpgradeLocal::default()),
            server_version: api::ServerVersion::default(),
            upgrade_polling_paused: AtomicBool::new(false),
            registry: Mutex::new(registry_mirror::RegistryLocal::default()),
            takeover: Mutex::new(takeover::Slot::default()),
            starts: Mutex::new(HashMap::new()),
            data_dir: data_dir.to_owned(),
            previews: Mutex::new(HashMap::new()),
            confirmations: Mutex::new(VecDeque::new()),
            ports,
            media,
        });
        link.restore_core_upgrade();
        link.refresh_derived();
        runtime.spawn(follow_home(Arc::downgrade(&link)));
        link
    }

    pub(crate) fn view(&self) -> PluginsViewModel {
        self.view.borrow().clone()
    }

    /// 派生字段跟着输入重算：可更新的应用和侧边栏都由已安装列表、商店、任务忙闲和核心升级页算出来。
    /// 改了这些输入的地方要调一次；值没变时不打扰订阅者。
    pub(crate) fn refresh_derived(&self) {
        self.view.send_if_modified(|view| {
            let updatable = now::updatable(view);
            let mut changed = view.updatable != updatable;
            view.updatable = updatable;
            let sidebar = now::sidebar(view);
            changed |= view.sidebar != sidebar;
            view.sidebar = sidebar;
            changed
        });
    }

    /// 换服务器时新作用域接着用这个发送端，壳的订阅不断（ADR-0042）。
    pub(crate) fn sender(&self) -> watch::Sender<PluginsViewModel> {
        self.view.clone()
    }

    /// 调用方的运行时：聊天列表的安装行跟着插件状态走，也用它起订阅。
    pub(crate) fn runtime(&self) -> Handle {
        self.runtime.clone()
    }

    /// 现在记着的安装预览。
    pub(crate) fn previews(&self) -> HashMap<String, InstallPreview> {
        self.previews
            .lock()
            .expect("the previews lock never panics")
            .clone()
    }

    /// `SetPortMapping`：开关先存下，绑定的结果在 `port_mappings` 里。
    pub(crate) fn set_port_mapping(
        &self,
        plugin_id: String,
        enabled: bool,
    ) -> Result<(), CoreError> {
        self.ports.set_enabled(plugin_id, enabled)
    }

    /// `SetVisiblePlugin`：屏幕上换的是哪个插件的页面，`None` 是屏幕上没有插件页面。
    pub(crate) fn set_visible_plugin(&self, plugin_id: Option<String>) {
        self.ports.set_visible(plugin_id);
    }

    pub(crate) fn subscribe(&self) -> watch::Receiver<PluginsViewModel> {
        self.view.subscribe()
    }

    /// 壳告诉核心系统的语言偏好。已经取到的列表是按旧语言挑的，换了语言就重新取。
    pub(crate) fn set_languages(self: &Arc<Self>, languages: Vec<String>) {
        {
            let mut current = lock(&self.languages);
            if *current == languages {
                return;
            }
            *current = languages;
        }
        let (installed, store) = {
            let view = self.view.borrow();
            (view.installed.is_some(), view.store.is_some())
        };
        if installed {
            self.refresh_installed();
        }
        if store {
            self.refresh_store(false);
        }
    }

    pub(crate) fn foregrounded(self: &Arc<Self>) {
        if self.external {
            return;
        }
        self.foreground.store(true, Ordering::SeqCst);
        self.ports.set_foreground(true);
        self.poll_changed.notify_one();
        self.refresh_installed();
        self.fetch_core_upgrade();
    }

    /// 隧道正在建：这时不用发请求，连上后 [`follow_home`] 接着取。
    fn connecting(&self) -> bool {
        self.home.view().status == HomeStatus::Connecting
    }

    /// 关掉升级状态的定时轮询，已经开着的循环到下一次醒来为止。只有测试用。
    pub(crate) fn stop_upgrade_polling(&self) {
        self.upgrade_polling_paused.store(true, Ordering::SeqCst);
    }

    /// `RefreshPlugins`：在后台重新取已安装的插件。隧道断着时请求会触发重连，所以只在主人要看的时候发，隧道状态变化时不发。
    pub(crate) fn refresh_installed(self: &Arc<Self>) {
        if self.external || self.connecting() {
            return;
        }
        let link = self.clone();
        self.runtime.spawn(async move {
            link.home.retry_if_rejected().await;
            link.spawn_installed();
            link.spawn_storage();
            // 商店留下的问题，已安装列表取到了也清不掉，页面上的重试不连它一起取就没有用。
            if *lock(&link.problem_of) == Some(Listing::Store) {
                link.spawn_store(false);
            }
        });
    }

    /// `RefreshStore`：在后台取商店，`force` 时家里的服务器不等缓存过期。
    pub(crate) fn refresh_store(self: &Arc<Self>, force: bool) {
        self.store_wanted.store(true, Ordering::Relaxed);
        if self.external || self.connecting() {
            return;
        }
        let link = self.clone();
        self.runtime.spawn(async move {
            link.home.retry_if_rejected().await;
            link.spawn_store(force);
            if *lock(&link.problem_of) == Some(Listing::Installed) {
                link.spawn_installed();
            }
        });
    }

    fn spawn_installed(self: &Arc<Self>) {
        let link = self.clone();
        let generation = self.generation.load(Ordering::SeqCst);
        self.begin_refresh();
        self.runtime.spawn(async move {
            let result = link.fetch_installed().await;
            link.end_refresh(Listing::Installed, generation, result);
        });
    }

    fn spawn_store(self: &Arc<Self>, force: bool) {
        let link = self.clone();
        let generation = self.generation.load(Ordering::SeqCst);
        self.begin_refresh();
        self.runtime.spawn(async move {
            let result = link.fetch_store(force).await;
            link.end_refresh(Listing::Store, generation, result);
        });
    }

    fn begin_refresh(&self) {
        self.in_flight.fetch_add(1, Ordering::SeqCst);
        self.view.send_if_modified(|view| {
            let changed = !view.refreshing;
            view.refreshing = true;
            changed
        });
    }

    /// 其间换了服务器时，成败都不算数，不能清掉新服务器的问题。
    fn end_refresh(&self, listing: Listing, generation: u64, result: Result<(), PluginProblem>) {
        if self.in_flight.fetch_sub(1, Ordering::SeqCst) == 1 {
            self.view.send_modify(|view| view.refreshing = false);
        }
        if self.is_current(generation) {
            self.record(listing, result);
        }
    }

    /// 失败时记下原因，显示的还是上次取到的列表。列表连不上由 `HomeViewModel::notice` 负责说，连上后
    /// [`follow_home`] 自动再取（ADR-0032）；启动失败则必须明确显示，不能假装成功。成功只清同一个来源留下的问题，
    /// 避免后台轮询清掉商店或启动错误，也避免另一插件的启动成功覆盖失败。
    fn record(&self, listing: Listing, result: Result<(), PluginProblem>) {
        self.view.send_if_modified(|view| {
            let mut source = lock(&self.problem_of);
            match result {
                Err(problem)
                    if !matches!(listing, Listing::Start(_))
                        && problem.reason == PluginFailure::TransportUnavailable =>
                {
                    false
                }
                Ok(()) => {
                    let clears = view.problem.is_some() && source.as_ref() == Some(&listing);
                    if clears {
                        view.problem = None;
                        *source = None;
                    }
                    clears
                }
                Err(problem) => {
                    view.problem = Some(problem);
                    *source = Some(listing);
                    true
                }
            }
        });
    }

    /// 连不上时不知道现在还剩多少，不留着上次的数字。
    fn forget_storage(&self) {
        self.view.send_if_modified(|view| {
            let changed = view.storage.is_some() || view.storage_problem.is_some();
            view.storage = None;
            view.storage_problem = None;
            changed
        });
    }

    /// 配对到了另一台服务器：上一台的列表不能再显示；做完的任务不留，进行中的等不到结果了。还想看商店的话接着取，商店页可能
    /// 正开着。
    fn forget_server(&self) {
        let _registry = lock(&self.registry);
        self.generation.fetch_add(1, Ordering::SeqCst);
        lock(&self.starts).clear();
        cache::forget(&self.installed_cache);
        self.view.send_modify(|view| {
            *lock(&self.problem_of) = None;
            view.installed = None;
            view.store = None;
            view.problem = None;
            view.egress = EgressViewModel::default();
            view.registry_mirror = wonton_protocol::RegistryMirrorViewModel::default();
            view.core_upgrade = CoreUpgradeViewModel::default();
            view.storage = None;
            view.storage_problem = None;
            view.tasks
                .retain(|task| task.state == PluginTaskState::Running);
            for task in &mut view.tasks {
                task.state = tasks::server_changed();
            }
        });
        self.forget_core_upgrade();
        self.forget_takeover();
    }

    fn is_current_home(&self, generation: u64, server: &Option<String>) -> bool {
        self.is_current(generation) && self.home.view().server_id == *server
    }

    fn is_current(&self, generation: u64) -> bool {
        self.generation.load(Ordering::SeqCst) == generation
    }

    /// 取磁盘空间放进 ViewModel。连不上不算它的问题：主屏的连接那一行已经说了。
    fn spawn_storage(self: &Arc<Self>) {
        let generation = self.generation.load(Ordering::SeqCst);
        let link = Arc::clone(self);
        self.runtime.spawn(async move {
            let fetched: Result<StorageUsage, PluginProblem> = match link.api() {
                Ok(api) => api.get(&["storage"], &[], READ_TIMEOUT).await,
                Err(problem) => Err(problem),
            };
            if !link.is_current(generation) {
                return;
            }
            link.view.send_modify(|view| match fetched {
                Ok(usage) => {
                    view.storage = Some(usage);
                    view.storage_problem = None;
                }
                Err(problem) => {
                    view.storage = None;
                    view.storage_problem =
                        (problem.reason != PluginFailure::TransportUnavailable).then_some(problem);
                }
            });
        });
    }

    /// 取已安装列表放进 ViewModel，有插件正在变化时开始盯着。其间换了服务器时，成败都不算数。
    async fn fetch_installed(self: &Arc<Self>) -> Result<(), PluginProblem> {
        let generation = self.generation.load(Ordering::SeqCst);
        let server = self.home.view().server_id;
        let api = self.api()?;
        self.fetch_installed_from(&api, generation, &server).await
    }

    async fn fetch_installed_from(
        self: &Arc<Self>,
        api: &CoreApi,
        generation: u64,
        server: &Option<String>,
    ) -> Result<(), PluginProblem> {
        let result: Result<InstalledPlugins, PluginProblem> =
            api.get(&["plugins"], &[], READ_TIMEOUT).await;
        if !self.is_current_home(generation, server) {
            return Ok(());
        }
        let installed = result?;
        if self.view.borrow().installed.as_ref() != Some(&installed)
            && let Some(server) = server
        {
            cache::save(&self.installed_cache, server.clone(), installed.clone()).await;
        }
        if !self.is_current_home(generation, server) {
            return Ok(());
        }
        self.view
            .send_modify(|view| view.installed = Some(installed));
        self.refresh_derived();
        if self.needs_polling().is_some() {
            self.start_polling();
        }
        Ok(())
    }

    async fn list_installed(&self) -> Result<InstalledPlugins, PluginProblem> {
        self.api()?.get(&["plugins"], &[], READ_TIMEOUT).await
    }

    async fn fetch_store(&self, force: bool) -> Result<(), PluginProblem> {
        let generation = self.generation.load(Ordering::SeqCst);
        let query: &[(&str, &str)] = if force { &[("refresh", "true")] } else { &[] };
        let result: Result<StoreListing, PluginProblem> =
            async { self.api()?.get(&["store"], query, SOURCE_TIMEOUT).await }.await;
        if !self.is_current(generation) {
            return Ok(());
        }
        let store = result?;
        self.view.send_modify(|view| view.store = Some(store));
        self.refresh_derived();
        Ok(())
    }

    pub(crate) fn api(&self) -> Result<CoreApi, PluginProblem> {
        let proxy = self.home.view().proxy.ok_or_else(api::not_connected)?;
        let base = self.home.core_url().ok_or_else(api::not_connected)?;
        let client = {
            let mut cached = lock(&self.client);
            match cached.as_ref() {
                Some((settings, client)) if *settings == proxy => client.clone(),
                _ => {
                    let client = proxied_client(&proxy)?;
                    *cached = Some((proxy, client.clone()));
                    client
                }
            }
        };
        let languages = lock(&self.languages)
            .iter()
            .filter(|tag| tag.is_ascii())
            .cloned()
            .collect::<Vec<_>>()
            .join(",");
        self.server_version
            .seed(lock(&self.upgrade).running_version());
        Ok(CoreApi {
            version: self.server_version.clone(),
            base,
            client,
            languages,
        })
    }

    pub(crate) async fn store_plugin(&self, source: &str) -> Result<PluginDetail, CoreError> {
        let api = self.api().map_err(to_core_error)?;
        api.get(&["store", "plugin"], &[("source", source)], SOURCE_TIMEOUT)
            .await
            .map_err(to_core_error)
    }

    pub(crate) async fn installed_plugin(&self, id: &str) -> Result<PluginDetail, CoreError> {
        let api = self.api().map_err(to_core_error)?;
        api.get(&["plugins", id], &[], READ_TIMEOUT)
            .await
            .map_err(to_core_error)
    }

    pub(crate) async fn preview_install(
        &self,
        source: String,
    ) -> Result<InstallPreview, CoreError> {
        let api = self.api().map_err(to_core_error)?;
        api.post(
            &["plugins", "preview"],
            &InstallSource { source },
            SOURCE_TIMEOUT,
        )
        .await
        .map_err(to_core_error)
    }

    pub(crate) async fn preview_upgrade(&self, id: &str) -> Result<InstallPreview, CoreError> {
        let api = self.api().map_err(to_core_error)?;
        api.post(
            &["plugins", id, "upgrade", "preview"],
            &serde_json::json!({}),
            SOURCE_TIMEOUT,
        )
        .await
        .map_err(to_core_error)
    }

    /// 主人选的压缩包的预览：家里的服务器校验并暂存，预览的 `source` 就是它的来源。
    pub(crate) async fn preview_upload(
        &self,
        archive: Vec<u8>,
    ) -> Result<InstallPreview, CoreError> {
        let api = self.api().map_err(to_core_error)?;
        api.post_archive(&["plugins", "preview"], archive, SOURCE_TIMEOUT)
            .await
            .map_err(to_core_error)
    }

    /// 换版本的预览：换到主人选的来源（ADR-0031）。
    pub(crate) async fn preview_upgrade_from(
        &self,
        id: &str,
        source: String,
    ) -> Result<InstallPreview, CoreError> {
        let api = self.api().map_err(to_core_error)?;
        api.post(
            &["plugins", id, "upgrade", "preview"],
            &InstallSource { source },
            SOURCE_TIMEOUT,
        )
        .await
        .map_err(to_core_error)
    }

    /// 换版本的预览：换到主人选的压缩包（ADR-0031）。
    pub(crate) async fn preview_upgrade_upload(
        &self,
        id: &str,
        archive: Vec<u8>,
    ) -> Result<InstallPreview, CoreError> {
        let api = self.api().map_err(to_core_error)?;
        api.post_archive(
            &["plugins", id, "upgrade", "preview"],
            archive,
            SOURCE_TIMEOUT,
        )
        .await
        .map_err(to_core_error)
    }

    /// 添加来源：地址原样交给家里的服务器，它判断是插件还是索引。添加之后商店里多一段。
    pub(crate) async fn add_store_source(
        self: &Arc<Self>,
        address: String,
    ) -> Result<AddedSource, CoreError> {
        let api = self.api().map_err(to_core_error)?;
        let added = api
            .post(
                &["store", "sources"],
                &AddSourceRequest { address },
                SOURCE_TIMEOUT,
            )
            .await
            .map_err(to_core_error)?;
        self.refresh_store(false);
        Ok(added)
    }

    /// 删除来源，只影响发现，已安装的插件不动。`address` 是 [`AddedSource::address`]。
    pub(crate) async fn remove_store_source(
        self: &Arc<Self>,
        address: &str,
    ) -> Result<(), CoreError> {
        let api = self.api().map_err(to_core_error)?;
        api.delete(&["store", "sources"], &[("address", address)], READ_TIMEOUT)
            .await
            .map_err(to_core_error)?;
        self.refresh_store(false);
        Ok(())
    }

    pub(crate) async fn set_lan_webui(
        self: &Arc<Self>,
        id: &str,
        enabled: bool,
    ) -> Result<PluginDetail, CoreError> {
        let api = self.api().map_err(to_core_error)?;
        let update = LanWebuiUpdate { enabled };
        let result = api
            .put(&["plugins", id, "lan-webui"], &update, INSTALL_TIMEOUT)
            .await
            .map_err(to_core_error);
        self.refresh_installed();
        result
    }

    pub(crate) async fn set_plugin_permission(
        self: &Arc<Self>,
        id: &str,
        permission: &str,
        enabled: bool,
    ) -> Result<PluginDetail, CoreError> {
        let api = self.api().map_err(to_core_error)?;
        let update = PermissionUpdate { enabled };
        let result = api
            .put(
                &["plugins", id, "permissions", permission],
                &update,
                INSTALL_TIMEOUT,
            )
            .await
            .map_err(to_core_error);
        self.refresh_installed();
        result
    }

    pub(crate) async fn set_ask_every_update(
        &self,
        id: &str,
        enabled: bool,
    ) -> Result<PluginDetail, CoreError> {
        let api = self.api().map_err(to_core_error)?;
        let update = AskEveryUpdate { enabled };
        api.put(&["plugins", id, "ask-every-update"], &update, READ_TIMEOUT)
            .await
            .map_err(to_core_error)
    }

    pub(crate) async fn revoke_save_grant(
        &self,
        id: &str,
        folder: &str,
    ) -> Result<PluginDetail, CoreError> {
        let api = self.api().map_err(to_core_error)?;
        api.delete(
            &["plugins", id, "save-grants"],
            &[("folder", folder)],
            READ_TIMEOUT,
        )
        .await
        .map_err(to_core_error)?;
        self.installed_plugin(id).await
    }

    pub(crate) async fn load_image(&self, image: &PluginImage) -> Result<Vec<u8>, CoreError> {
        let cached = self.images_dir.join(cache_name(&image.path));
        if let Ok(bytes) = tokio::fs::read(&cached).await {
            return Ok(bytes);
        }
        let api = self.api().map_err(to_core_error)?;
        let bytes = api.image(image).await.map_err(to_core_error)?;
        if let Err(error) = save_atomically(&self.images_dir, &cached, &bytes).await {
            tracing::warn!("can't write the app image to the cache: {error}");
        }
        Ok(bytes)
    }

    /// 在取到的商店列表里找：名字、说明、标签和 ID，不分大小写，官方的在前。
    pub(crate) fn search_store(&self, query: &str) -> Vec<StoreItem> {
        let view = self.view.borrow();
        let Some(store) = view.store.as_ref() else {
            return Vec::new();
        };
        let query = query.trim().to_lowercase();
        store
            .official
            .plugins
            .iter()
            .chain(
                store
                    .added
                    .iter()
                    .flat_map(|source| &source.section.plugins),
            )
            .chain(&store.community.plugins)
            .filter(|item| {
                item.title.to_lowercase().contains(&query)
                    || item.summary.to_lowercase().contains(&query)
                    || item.id.to_lowercase().contains(&query)
                    || item
                        .tags
                        .iter()
                        .any(|tag| tag.to_lowercase().contains(&query))
            })
            .cloned()
            .collect()
    }
}
