//! 服务器作用域（ADR-0042）：连着一台服务器的那些部件和它们的运行时。切换服务器就是关掉一个、按另一个槽位建一个。

mod runtime;
mod visit;

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use tokio::sync::watch;
use wonton_protocol::{
    ChatViewModel, CoreError, FriendsViewModel, HomeViewModel, OnboardingViewModel, PeersViewModel,
    PluginsViewModel, RoomPanel, ServersViewModel, SettingsViewModel, TabsViewModel,
    TransfersViewModel,
};

use crate::chat::{CardSuccess, ChatLink};
use crate::files::FilesLink;
use crate::friends::FriendsLink;
use crate::home::{HomeLink, Network, Storage};
use crate::onboarding::OnboardingLink;
use crate::peers::PeersLink;
use crate::plugins::PluginsLink;
use crate::push::PushLink;
use crate::push_method::PushMethodLink;
use crate::settings::SettingsLink;
use crate::tabs::TabsLink;
pub(crate) use runtime::ScopeRuntime;
pub(crate) use visit::{Entered, Visits};
use wonton_matrix::{MatrixSession, OwnedRoomId};

pub(crate) struct Scope {
    pub(crate) slot: String,
    pub(crate) home: Arc<HomeLink>,
    pub(crate) chat: Arc<ChatLink>,
    pub(crate) push: Arc<PushLink>,
    pub(crate) push_method: PushMethodLink,
    pub(crate) plugins: Arc<PluginsLink>,
    pub(crate) tabs: Arc<TabsLink>,
    pub(crate) files: Arc<FilesLink>,
    pub(crate) onboarding: Arc<OnboardingLink>,
    pub(crate) peers: Arc<PeersLink>,
    pub(crate) friends: Arc<FriendsLink>,
    pub(crate) settings: SettingsLink,
    /// 卡片成功的测试钩子，只给集成测试用（feature `test-hooks`，生产构建没有）。挂在作用域上：
    /// 换服务器换作用域，测试不换服务器就一直是同一个。
    #[cfg(feature = "test-hooks")]
    pub(crate) card_success_hook: Arc<crate::test_hooks::CardSuccessHook>,
    runtime: ScopeRuntime,
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
    pub(crate) peers: watch::Sender<PeersViewModel>,
    pub(crate) friends: watch::Sender<FriendsViewModel>,
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
            peers: scope.peers.sender(),
            friends: scope.friends.sender(),
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

/// 建各个联系模块时共用的东西（作用域自己的句柄、已有链接和目录）。
struct Shared<'a> {
    handle: &'a tokio::runtime::Handle,
    home: &'a Arc<HomeLink>,
    plugins: &'a Arc<PluginsLink>,
    push_method: &'a PushMethodLink,
    dir: &'a Path,
    views: Option<&'a Views>,
}

/// 聊天这一支建出来的链接：聊天、推送、底栏、文件和引导。
struct ChatLinks {
    chat: Arc<ChatLink>,
    push: Arc<PushLink>,
    tabs: Arc<TabsLink>,
    files: Arc<FilesLink>,
    onboarding: Arc<OnboardingLink>,
}

fn chat_links(
    shared: &Shared<'_>,
    card_success: CardSuccess,
    channels: crate::chat::ChatChannels,
    session_in: watch::Receiver<Option<Arc<MatrixSession>>>,
    room_sender: watch::Sender<Option<(OwnedRoomId, RoomPanel)>>,
    room_receiver: watch::Receiver<Option<(OwnedRoomId, RoomPanel)>>,
) -> Result<ChatLinks, CoreError> {
    let Shared {
        handle,
        home,
        plugins,
        push_method,
        dir,
        views,
    } = *shared;
    let chat = ChatLink::new(
        handle.clone(),
        home.clone(),
        dir,
        card_success,
        plugins.clone(),
        channels,
    )?;
    let push = PushLink::new(
        handle.clone(),
        home.clone(),
        chat.clone(),
        dir,
        push_method.subscribe(),
    );
    let tabs = TabsLink::new(
        handle.clone(),
        home.clone(),
        plugins.clone(),
        dir,
        views.map(|v| v.tabs.clone()),
        session_in,
        (room_sender, room_receiver),
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
    Ok(ChatLinks {
        chat,
        push,
        tabs,
        files,
        onboarding,
    })
}

fn people_links(
    shared: &Shared<'_>,
    servers: watch::Receiver<ServersViewModel>,
    chat: &Arc<ChatLink>,
) -> Result<(Arc<PeersLink>, Arc<FriendsLink>, SettingsLink), CoreError> {
    let Shared {
        handle,
        home,
        plugins,
        push_method,
        dir,
        views,
    } = *shared;
    let peers = PeersLink::new(handle.clone(), home.clone(), views.map(|v| v.peers.clone()));
    let friends = FriendsLink::new(
        handle.clone(),
        home.clone(),
        chat.clone(),
        dir,
        views.map(|v| v.friends.clone()),
    )?;
    let settings = SettingsLink::new(
        handle,
        home.subscribe(),
        chat.subscribe(),
        plugins.subscribe(),
        peers.subscribe(),
        friends.subscribe(),
        servers,
        push_method.subscribe(),
        views.map(|v| v.settings.clone()),
    );
    Ok((peers, friends, settings))
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
        let runtime = ScopeRuntime::new();
        let handle = runtime.handle().expect("the runtime was just built");
        // 会话和打开着的房间的面板（ADR-0052）：chat 往里喂，tabs 用来同步账号数据、合成房间面板的视图。
        let session_out = watch::channel(None);
        let room_panel = watch::channel(None);
        let storage = Storage::open(root, dir)?;
        // 方式是整机设置（设置页一个开关管所有服务器），放在设备级的 root，不放每台服务器一个的槽位目录。
        let push_method = PushMethodLink::new(root);
        let home = HomeLink::new(
            handle.clone(),
            storage,
            network,
            views.map(|v| v.home.clone()),
            push_method.subscribe(),
        )?;
        let plugins = PluginsLink::new(
            handle.clone(),
            home.clone(),
            dir,
            views.map(|v| v.plugins.clone()),
        );
        #[cfg(feature = "test-hooks")]
        let card_success_hook = Arc::new(crate::test_hooks::CardSuccessHook::new());
        // 测试钩子开着时决定先报到、等放行（见 test_hooks），settled 也就发不出去；生产构建直通。
        #[cfg(feature = "test-hooks")]
        let card_success = crate::test_hooks::wrap(plugins.clone(), card_success_hook.clone());
        #[cfg(not(feature = "test-hooks"))]
        let card_success = {
            let plugins = plugins.clone();
            Arc::new(move |plugin_id: &str| plugins.card_succeeded(plugin_id))
        };
        let channels = crate::chat::ChatChannels {
            view: views.map(|v| v.chat.clone()),
            method: push_method.subscribe(),
            session_out: session_out.0.clone(),
            room_panel_out: room_panel.0.clone(),
        };
        let shared = Shared {
            handle: &handle,
            home: &home,
            plugins: &plugins,
            push_method: &push_method,
            dir,
            views,
        };
        let links = chat_links(
            &shared,
            card_success,
            channels,
            session_out.1,
            room_panel.0,
            room_panel.1,
        )?;
        let people = people_links(&shared, servers, &links.chat)?;
        Ok(Arc::new(Self {
            slot: slot.to_owned(),
            home,
            chat: links.chat,
            push: links.push,
            push_method,
            plugins,
            tabs: links.tabs,
            files: links.files,
            onboarding: links.onboarding,
            peers: people.0,
            friends: people.1,
            settings: people.2,
            #[cfg(feature = "test-hooks")]
            card_success_hook,
            runtime,
        }))
    }

    pub(crate) fn foregrounded(&self) {
        self.home.foregrounded();
        self.chat.app_foregrounded();
        self.push.foregrounded();
        self.plugins.foregrounded();
        self.files.foregrounded();
    }

    /// 壳给的语言交给作用域里每个按 App 语言写字或把语言带给家里服务器（Accept-Language）的模块。
    /// 壳发 `SetLanguages` 和换作用域时重放都走这里，两处不各列一份，漏不掉哪个模块。
    pub(crate) fn set_languages(&self, languages: &[String]) {
        self.plugins.set_languages(languages.to_vec());
        self.onboarding.set_languages(languages.to_vec());
        self.peers.set_languages(languages.to_vec());
    }

    pub(crate) fn backgrounded(&self) {
        self.home.backgrounded();
        self.chat.backgrounded();
        self.plugins.backgrounded();
    }

    pub(crate) async fn run<T: Send + 'static>(
        &self,
        task: impl Future<Output = Result<T, CoreError>> + Send + 'static,
    ) -> Result<T, CoreError> {
        self.runtime.run(task).await
    }

    pub(crate) fn spawn(&self, task: impl Future<Output = ()> + Send + 'static) {
        self.runtime.spawn(task);
    }

    /// 停掉这台服务器：进行中的传输记成暂停，后台任务一起结束。
    ///
    /// 换服务器时（`wait`）等旧任务都停下：它们和新作用域共用壳订阅的发送端，停下前最后的发送、任务析构时的发送不能
    /// 盖掉新服务器的状态（实测过：不等的话，连上新服务器后连接状态会被旧的“未配对”盖掉）。核心释放时不等：壳可能在
    /// 主线程上释放核心。
    pub(crate) fn close(&self, wait: bool) {
        self.files.close();
        if wait {
            self.runtime.stop();
        } else {
            self.runtime.stop_background();
        }
    }
}
