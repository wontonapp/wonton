//! 服务器作用域（ADR-0042）：连着一台服务器的那些部件和它们的运行时。切换服务器就是关掉一个、按另一个槽位建一个。

use std::future::Future;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use tokio::runtime::{Handle, Runtime};
use tokio::sync::watch;
use wonton_protocol::{
    ChatViewModel, CoreError, HomeViewModel, OnboardingViewModel, PluginsViewModel,
    ServersViewModel, SettingsViewModel, TabsViewModel, TransfersViewModel,
};

use crate::chat::ChatLink;
use crate::files::FilesLink;
use crate::home::{HomeLink, Network, Storage};
use crate::onboarding::OnboardingLink;
use crate::plugins::PluginsLink;
use crate::push::PushLink;
use crate::settings::SettingsLink;
use crate::tabs::TabsLink;

pub(crate) struct Scope {
    pub(crate) slot: String,
    pub(crate) home: Arc<HomeLink>,
    pub(crate) chat: Arc<ChatLink>,
    pub(crate) push: Arc<PushLink>,
    pub(crate) plugins: Arc<PluginsLink>,
    pub(crate) tabs: Arc<TabsLink>,
    pub(crate) files: Arc<FilesLink>,
    pub(crate) onboarding: Arc<OnboardingLink>,
    pub(crate) settings: SettingsLink,
    runtime: Mutex<Option<Runtime>>,
}

/// 壳订阅的那些 ViewModel 的发送端。第一个作用域新建，之后每个作用域接着用（ADR-0042）：壳拿到的订阅不用换，
/// 切换后收到的就是另一台服务器的状态。
#[derive(Clone)]
pub(crate) struct Views {
    pub(crate) home: watch::Sender<HomeViewModel>,
    pub(crate) chat: watch::Sender<ChatViewModel>,
    pub(crate) plugins: watch::Sender<PluginsViewModel>,
    pub(crate) tabs: watch::Sender<TabsViewModel>,
    pub(crate) transfers: watch::Sender<TransfersViewModel>,
    pub(crate) onboarding: watch::Sender<OnboardingViewModel>,
    pub(crate) settings: watch::Sender<SettingsViewModel>,
}

impl Views {
    pub(crate) fn of(scope: &Scope) -> Self {
        Self {
            home: scope.home.sender(),
            chat: scope.chat.sender(),
            plugins: scope.plugins.sender(),
            tabs: scope.tabs.sender(),
            transfers: scope.files.sender(),
            onboarding: scope.onboarding.sender(),
            settings: scope.settings.sender(),
        }
    }
}

/// 部件的 ViewModel 发送端：有现成的（换服务器）就接着用，发出新作用域的初始值；没有就新建。
pub(crate) fn reuse<T>(sender: Option<watch::Sender<T>>, initial: T) -> watch::Sender<T> {
    match sender {
        Some(sender) => {
            sender.send_replace(initial);
            sender
        }
        None => watch::Sender::new(initial),
    }
}

impl Scope {
    /// `root` 是壳给的数据目录（放设备密钥），`dir` 是这个槽位的目录。`views` 是上一个作用域留下的发送端，
    /// 上一个作用域要先关掉：不然它停下前的最后几次发送会盖掉新作用域的状态。
    pub(crate) fn open(
        slot: &str,
        root: &Path,
        dir: &Path,
        network: Network,
        servers: watch::Receiver<ServersViewModel>,
        views: Option<&Views>,
    ) -> Result<Arc<Self>, CoreError> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("wonton-core")
            .enable_all()
            .build()
            .expect("创建 tokio 运行时失败");
        let handle = runtime.handle().clone();
        let storage = Storage::open(root, dir)?;
        let home = HomeLink::new(
            handle.clone(),
            storage,
            network,
            views.map(|v| v.home.clone()),
        )?;
        let plugins = PluginsLink::new(
            handle.clone(),
            home.clone(),
            dir,
            views.map(|v| v.plugins.clone()),
        );
        let card_success = {
            let plugins = plugins.clone();
            Arc::new(move |plugin_id: &str| plugins.card_succeeded(plugin_id))
        };
        let chat = ChatLink::new(
            handle.clone(),
            home.clone(),
            dir,
            card_success,
            plugins.clone(),
            views.map(|v| v.chat.clone()),
        )?;
        let push = PushLink::new(handle.clone(), home.clone(), chat.clone());
        let tabs = TabsLink::new(
            handle.clone(),
            home.clone(),
            plugins.clone(),
            dir,
            views.map(|v| v.tabs.clone()),
        );
        let files = FilesLink::new(
            handle.clone(),
            home.clone(),
            chat.clone(),
            dir,
            views.map(|v| v.transfers.clone()),
        );
        let onboarding = OnboardingLink::new(
            handle.clone(),
            home.clone(),
            chat.clone(),
            plugins.clone(),
            dir,
            views.map(|v| v.onboarding.clone()),
        );
        let settings = SettingsLink::new(
            &handle,
            home.subscribe(),
            chat.subscribe(),
            plugins.subscribe(),
            servers,
            views.map(|v| v.settings.clone()),
        );
        Ok(Arc::new(Self {
            slot: slot.to_owned(),
            home,
            chat,
            push,
            plugins,
            tabs,
            files,
            onboarding,
            settings,
            runtime: Mutex::new(Some(runtime)),
        }))
    }

    pub(crate) fn foregrounded(&self) {
        self.home.foregrounded();
        self.chat.app_foregrounded();
        self.push.foregrounded();
        self.plugins.foregrounded();
        self.files.foregrounded();
    }

    pub(crate) fn backgrounded(&self) {
        self.home.backgrounded();
        self.chat.backgrounded();
        self.plugins.backgrounded();
    }

    fn handle(&self) -> Option<Handle> {
        let runtime = lock(&self.runtime);
        runtime.as_ref().map(|runtime| runtime.handle().clone())
    }

    /// 在这个作用域的运行时里执行：壳（Swift / Kotlin）驱动导出的 async 方法时，线程上没有 tokio 运行时，
    /// matrix-sdk 的网络操作离不开它。
    pub(crate) async fn run<T: Send + 'static>(
        &self,
        task: impl Future<Output = Result<T, CoreError>> + Send + 'static,
    ) -> Result<T, CoreError> {
        let handle = self.handle().ok_or_else(switched)?;
        handle.spawn(task).await.map_err(|error| {
            if error.is_cancelled() {
                switched()
            } else {
                CoreError::Upstream {
                    reason: format!("核心的后台任务异常结束：{error}"),
                }
            }
        })?
    }

    pub(crate) fn spawn(&self, task: impl Future<Output = ()> + Send + 'static) {
        if let Some(handle) = self.handle() {
            handle.spawn(task);
        }
    }

    /// 停掉这台服务器：进行中的传输记成暂停，后台任务一起结束。
    ///
    /// 换服务器时（`wait`）等旧任务都停下，最多一秒：它们和新作用域共用壳订阅的发送端，停下前最后的发送、任务析构时
    /// 的发送不能盖掉新服务器的状态（实测过：不等的话，连上新服务器后连接状态会被旧的“未配对”盖掉）。在另一个线程上等，
    /// 调用方可能在异步上下文里，那里不能阻塞地关运行时。核心释放时不等：壳可能在主线程上释放核心。
    pub(crate) fn close(&self, wait: bool) {
        self.files.close();
        let Some(runtime) = lock(&self.runtime).take() else {
            return;
        };
        if wait {
            let stopping = std::thread::spawn(move || runtime.shutdown_timeout(STOP_TIMEOUT));
            if stopping.join().is_err() {
                tracing::warn!("旧服务器的运行时没能正常停下");
            }
        } else {
            runtime.shutdown_background();
        }
    }
}

const STOP_TIMEOUT: Duration = Duration::from_secs(1);

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn switched() -> CoreError {
    CoreError::NotReady {
        reason: "已经切到另一台服务器".to_owned(),
    }
}
