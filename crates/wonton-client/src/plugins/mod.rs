//! 插件页面的状态（ADR-0017 的“客户端核心”）：已安装的插件、商店、取列表的问题，以及安装、升级、卸载的进展。文字由
//! 家里的服务器按主人的语言挑好，这里原样放进 ViewModel。

mod api;
mod egress;
mod tasks;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::Duration;

use reqwest::Client;
use tokio::runtime::Handle;
use tokio::sync::watch;
use wonton_protocol::{
    CoreError, EgressViewModel, HomeFailure, HomeStatus, InstallPreview, InstallSource,
    InstalledPlugins, LanWebuiUpdate, PluginDetail, PluginFailure, PluginImage, PluginProblem,
    PluginState, PluginTaskState, PluginsViewModel, ProxySettings, StoreItem, StoreListing,
};

use crate::home::HomeLink;
use api::{CoreApi, INSTALL_TIMEOUT, READ_TIMEOUT, SOURCE_TIMEOUT, proxied_client, to_core_error};

/// 有插件在安装、升级、卸载时，隔这么久重新取一次已安装列表。
const POLL_INTERVAL: Duration = Duration::from_secs(3);

/// `PluginsViewModel.problem` 是取哪个列表时留下的。
#[derive(Clone, Copy, PartialEq, Eq)]
enum Listing {
    Installed,
    Store,
}

pub(crate) struct PluginsLink {
    runtime: Handle,
    home: Arc<HomeLink>,
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
    /// `view.problem` 来自哪个列表；为空时来自隧道，哪个列表取到了都说明它过去了。
    problem_of: Mutex<Option<Listing>>,
    /// 每换一台服务器加一：之前发出的请求和任务，结果不再算数。
    generation: AtomicU64,
    /// 每次取或改出网代理的设置加一：只有最后发起的那次能写 `view.egress`。
    egress_epoch: AtomicU64,
}

impl PluginsLink {
    pub(crate) fn new(runtime: Handle, home: Arc<HomeLink>, data_dir: &Path) -> Arc<Self> {
        let link = Arc::new(Self {
            runtime: runtime.clone(),
            home,
            view: watch::Sender::new(PluginsViewModel::default()),
            languages: Mutex::new(Vec::new()),
            client: Mutex::new(None),
            images_dir: data_dir.join("plugin-images"),
            in_flight: AtomicUsize::new(0),
            store_wanted: AtomicBool::new(false),
            polling: AtomicBool::new(false),
            problem_of: Mutex::new(None),
            generation: AtomicU64::new(0),
            egress_epoch: AtomicU64::new(0),
        });
        runtime.spawn(follow_home(Arc::downgrade(&link)));
        link
    }

    pub(crate) fn view(&self) -> PluginsViewModel {
        self.view.borrow().clone()
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
        self.refresh_installed();
    }

    /// 隧道正在建：这时的刷新不算失败，连上后 [`follow_home`] 接着取，连不上时它也会报原因。
    fn connecting(&self) -> bool {
        self.home.view().status == HomeStatus::Connecting
    }

    /// `RefreshPlugins`：在后台重新取已安装的插件。隧道断着时请求会触发重连，所以只在主人要看的时候发，隧道状态变化时不发。
    pub(crate) fn refresh_installed(self: &Arc<Self>) {
        if self.connecting() {
            return;
        }
        self.spawn_installed();
        // 商店留下的问题，已安装列表取到了也清不掉，页面上的重试不连它一起取就没有用。
        if *lock(&self.problem_of) == Some(Listing::Store) {
            self.spawn_store(false);
        }
    }

    /// `RefreshStore`：在后台取商店，`force` 时家里的服务器不等缓存过期。
    pub(crate) fn refresh_store(self: &Arc<Self>, force: bool) {
        self.store_wanted.store(true, Ordering::Relaxed);
        if self.connecting() {
            return;
        }
        self.spawn_store(force);
        if *lock(&self.problem_of) == Some(Listing::Installed) {
            self.spawn_installed();
        }
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

    /// 失败时记下原因，显示的还是上次取到的列表。`problem` 只有一个字段，两个列表共用，成功只清自己留下的：否则有插件在装、
    /// 每 3 秒取一次已安装列表时，商店取不到的原因马上就没了，商店页一直转圈。
    fn record(&self, listing: Listing, result: Result<(), PluginProblem>) {
        self.view.send_if_modified(|view| {
            let mut source = lock(&self.problem_of);
            match result {
                Ok(()) => {
                    let clears =
                        view.problem.is_some() && source.is_none_or(|source| source == listing);
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

    /// 隧道建不起来：原因直接写进去。不能靠发请求来报，请求本身又会触发重连。
    fn report_home_failure(&self, problem: PluginProblem) {
        self.view.send_if_modified(|view| {
            if view.problem.as_ref() == Some(&problem) {
                return false;
            }
            view.problem = Some(problem);
            *lock(&self.problem_of) = None;
            true
        });
    }

    /// 配对到了另一台服务器：上一台的列表不能再显示；做完的任务不留，进行中的等不到结果了。还想看商店的话接着取，商店页可能
    /// 正开着。
    fn forget_server(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.view.send_modify(|view| {
            *lock(&self.problem_of) = None;
            view.installed = None;
            view.store = None;
            view.problem = None;
            view.egress = EgressViewModel::default();
            view.tasks
                .retain(|task| task.state == PluginTaskState::Running);
            for task in &mut view.tasks {
                task.state = tasks::server_changed();
            }
        });
    }

    fn is_current(&self, generation: u64) -> bool {
        self.generation.load(Ordering::SeqCst) == generation
    }

    /// 取已安装列表放进 ViewModel，有插件正在变化时开始盯着。其间换了服务器时，成败都不算数。
    async fn fetch_installed(self: &Arc<Self>) -> Result<(), PluginProblem> {
        let generation = self.generation.load(Ordering::SeqCst);
        let result = self.list_installed().await;
        if !self.is_current(generation) {
            return Ok(());
        }
        let installed = result?;
        self.view
            .send_modify(|view| view.installed = Some(installed));
        if self.needs_polling() {
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
        Ok(())
    }

    /// 还有插件没到稳定状态，或者任务没做完。
    fn needs_polling(&self) -> bool {
        let view = self.view.borrow();
        let settling = view.installed.as_ref().is_some_and(|installed| {
            installed.plugins.iter().any(|plugin| {
                matches!(
                    plugin.status,
                    PluginState::Installing | PluginState::Upgrading | PluginState::Uninstalling
                )
            })
        });
        settling
            || view
                .tasks
                .iter()
                .any(|task| task.state == PluginTaskState::Running)
    }

    /// 全新主机上 Hermes 要装好几分钟，这期间主屏上它的状态要跟着变，不靠主人下拉刷新。连不上时停下（任务还在做时除外），
    /// 隧道连上后由 [`follow_home`] 接着。
    fn start_polling(self: &Arc<Self>) {
        if self.polling.swap(true, Ordering::SeqCst) {
            return;
        }
        let link = self.clone();
        self.runtime.spawn(async move {
            let unreachable = loop {
                tokio::time::sleep(POLL_INTERVAL).await;
                if !link.needs_polling() {
                    break false;
                }
                let result = link.poll_once().await;
                if result.is_err() && !link.has_running_task() {
                    break true;
                }
            };
            link.polling.store(false, Ordering::SeqCst);
            // 决定退出之后才开始的任务或安装，那时看到轮询还在，没有再启动。因为连不上才退出的不算：接着盯就是在反复重连。
            if link.has_running_task() || (!unreachable && link.needs_polling()) {
                link.start_polling();
            }
        });
    }

    fn has_running_task(&self) -> bool {
        self.view
            .borrow()
            .tasks
            .iter()
            .any(|task| task.state == PluginTaskState::Running)
    }

    async fn poll_once(&self) -> Result<(), PluginProblem> {
        let generation = self.generation.load(Ordering::SeqCst);
        let result = self.list_installed().await;
        if !self.is_current(generation) {
            return Ok(());
        }
        let outcome = result.map(|installed| {
            self.view
                .send_modify(|view| view.installed = Some(installed));
        });
        self.record(Listing::Installed, outcome.clone());
        outcome
    }

    fn api(&self) -> Result<CoreApi, PluginProblem> {
        let not_connected = || PluginProblem {
            reason: PluginFailure::TransportUnavailable,
            message: "还没连上家里的服务器".to_owned(),
        };
        let proxy = self.home.view().proxy.ok_or_else(not_connected)?;
        let base = self.home.core_url().ok_or_else(not_connected)?;
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
        Ok(CoreApi {
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

    /// 图片的地址内容不变（ADR-0017），取到一次就存在本机。缓存写不进去不影响这次的结果，下次再取。
    pub(crate) async fn load_image(&self, image: &PluginImage) -> Result<Vec<u8>, CoreError> {
        let cached = self.images_dir.join(cache_name(&image.path));
        if let Ok(bytes) = tokio::fs::read(&cached).await {
            return Ok(bytes);
        }
        let api = self.api().map_err(to_core_error)?;
        let bytes = api.image(image).await.map_err(to_core_error)?;
        if let Err(error) = save_atomically(&self.images_dir, &cached, &bytes).await {
            tracing::warn!("插件图片写不进缓存：{error}");
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

/// 跟着家里的连接：连上（App 启动、重新配对、断线恢复）时取列表，壳在页面出现时发的刷新可能比隧道早；连不上时报原因；换了
/// 服务器时忘掉上一台的东西。
async fn follow_home(link: Weak<PluginsLink>) {
    let Some(mut home) = link.upgrade().map(|link| link.home.subscribe()) else {
        return;
    };
    let mut connected = false;
    let mut server = home.borrow().server_id.clone();
    loop {
        let (now_connected, next_server, failure) = {
            let view = home.borrow_and_update();
            let failure = match &view.status {
                HomeStatus::Disconnected { reason, message } => Some(PluginProblem {
                    reason: home_failure(*reason),
                    message: message.clone(),
                }),
                _ => None,
            };
            (
                view.status == HomeStatus::Connected,
                view.server_id.clone(),
                failure,
            )
        };
        let Some(link) = link.upgrade() else { return };
        if server.is_some() && next_server != server {
            link.forget_server();
        }
        server = next_server;
        if let Some(failure) = failure {
            link.report_home_failure(failure);
        }
        if now_connected && !connected {
            link.refresh_installed();
            if link.store_wanted.load(Ordering::Relaxed) {
                link.refresh_store(false);
            }
        }
        connected = now_connected;
        drop(link);
        if home.changed().await.is_err() {
            return;
        }
    }
}

fn home_failure(reason: HomeFailure) -> PluginFailure {
    match reason {
        HomeFailure::Unauthorized => PluginFailure::Unauthorized,
        HomeFailure::TransportUnavailable => PluginFailure::TransportUnavailable,
        HomeFailure::UpstreamUnavailable => PluginFailure::Upstream,
        HomeFailure::Protocol => PluginFailure::Protocol,
    }
}

/// `/_wonton/api/images/<sha256>/<文件名>` 换成一个文件名。
fn cache_name(path: &str) -> String {
    path.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

async fn save_atomically(dir: &Path, target: &Path, bytes: &[u8]) -> std::io::Result<()> {
    // 同一张图可能同时在加载（主屏和详情页都要），各写各的临时文件。
    static NEXT: AtomicU64 = AtomicU64::new(0);
    tokio::fs::create_dir_all(dir).await?;
    let partial =
        target.with_extension(format!("{}.partial", NEXT.fetch_add(1, Ordering::Relaxed)));
    tokio::fs::write(&partial, bytes).await?;
    tokio::fs::rename(&partial, target).await
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
