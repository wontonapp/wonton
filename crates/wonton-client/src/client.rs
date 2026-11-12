use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use tokio::runtime::Runtime;
use tokio::sync::watch;
use wonton_protocol::{
    Action, AddedSource, AppearanceViewModel, ChatContact, ChatViewModel, CoreError, DeviceNetwork,
    FileCard, FileCardActions, FileFailure, FileId, FileInfo, FileOperation, FileProblem,
    HomeViewModel, InstallPreview, LanChoice, LanSettings, MediaHandle, OnboardingViewModel,
    OwnerProfile, OwnerProfileUpdate, PluginDetail, PluginImage, PluginsViewModel,
    PushNotification, RoomTransfers, ServersViewModel, SessionListViewModel, SettingsViewModel,
    StoreItem, TabRef, TabsViewModel, TransfersViewModel,
};

use crate::appearance::AppearanceLink;
use crate::chat::{ChatLink, TimelineSubscription};
use crate::files::{FileOpener, FilesLink};
use crate::home::{HomeLink, Network, Storage};
use crate::onboarding::OnboardingLink;
use crate::plugins::PluginsLink;
use crate::plugins::cards::follow_card_success;
use crate::push::PushLink;
use crate::servers::ServersLink;
use crate::session_store::SessionStore;
use crate::settings::SettingsLink;
use crate::subscription::{
    AppearanceSubscription, ChatSubscription, HomeSubscription, OnboardingSubscription,
    PluginsSubscription, ServersSubscription, SessionListSubscription, SettingsSubscription,
    TabsSubscription, TransfersSubscription,
};
use crate::tabs::TabsLink;
use crate::tabs::bridge::{self, TabBridgeResult, TabEnvironment};

mod tool_policy;

/// 客户端核心的入口。原生壳在启动时创建一个实例并一直持有。
///
/// 状态只存在这里：watch 通道既保存当前状态，也负责通知订阅者。网络操作在核心自己的 tokio 运行时
/// 里执行，进度和结果体现在 ViewModel 里。
#[derive(uniffi::Object)]
pub struct ClientCore {
    state: watch::Sender<SessionStore>,
    home: Arc<HomeLink>,
    chat: Arc<ChatLink>,
    push: Arc<PushLink>,
    plugins: Arc<PluginsLink>,
    tabs: Arc<TabsLink>,
    files: Arc<FilesLink>,
    appearance: AppearanceLink,
    onboarding: Arc<OnboardingLink>,
    settings: SettingsLink,
    servers: ServersLink,
    /// 网络操作用的运行时，随核心释放。
    runtime: Option<Runtime>,
}

#[uniffi::export]
impl ClientCore {
    /// `data_dir` 是 App 的私有数据目录（iOS 的 Application Support、Android 的 `filesDir`），
    /// 存设备密钥和上次的连接信息。上次连过家里的服务器时，创建后自动重连。
    #[uniffi::constructor]
    pub fn new(data_dir: String) -> Result<Arc<Self>, CoreError> {
        Self::with_network(&data_dir, Network::Internet)
    }

    /// 处理一个操作。会话操作成功时状态立即更新并推给所有订阅者；失败时状态不变，返回原因。
    ///
    /// 这个调用不阻塞，可以在主线程上直接调用。连接家里服务器这类网络操作在后台执行。
    pub fn dispatch(&self, action: Action) -> Result<(), CoreError> {
        match action {
            Action::CreateSession { title } => self.update_sessions(|store| store.create(&title)),
            Action::SelectSession { session_id } => {
                self.update_sessions(|store| store.select(session_id))
            }
            Action::DeleteSession { session_id } => {
                self.update_sessions(|store| store.delete(&session_id))
            }
            Action::ConnectHome { ticket } => self.home.connect(&ticket),
            Action::PairHome { code, device_name } => self.home.pair(code.expose(), &device_name),
            Action::InviteDevice => self.home.invite(),
            Action::AddServer
            | Action::CancelAddServer
            | Action::SwitchServer { .. }
            | Action::RenameServer { .. }
            | Action::RemoveServer { .. } => ServersLink::unsupported(),
            Action::AppForegrounded => {
                self.home.foregrounded();
                self.chat.foregrounded();
                self.push.foregrounded();
                self.plugins.foregrounded();
                self.files.foregrounded();
                Ok(())
            }
            Action::AppBackgrounded => {
                self.home.backgrounded();
                self.plugins.backgrounded();
                Ok(())
            }
            Action::NetworkChanged { network } => {
                self.home.network_changed(network);
                if network != DeviceNetwork::None {
                    // 连接可能没断、只是迁移了路径：同步和发送队列的退避不用等完。
                    self.chat.foregrounded();
                }
                Ok(())
            }
            Action::RetryConnection => self.home.retry(),
            Action::SetPushEndpoint { endpoint } => self.push.set_endpoint(endpoint),
            Action::PushUnavailable { message } => {
                self.push.unavailable(message);
                Ok(())
            }
            action @ (Action::RefreshOnboarding
            | Action::RestartOnboarding
            | Action::ChooseOnboardingAgent { .. }
            | Action::ContinueOnboarding
            | Action::SkipOnboardingStep
            | Action::BackOnboardingStep
            | Action::ToggleOnboardingNeed { .. }
            | Action::EditOnboardingProfile { .. }
            | Action::TestModelProvider { .. }
            | Action::UseCurrentModel
            | Action::ChooseModelLogin { .. }
            | Action::RetryOnboardingItem { .. }
            | Action::DismissModelProviderProblem
            | Action::SetPrimaryAgent { .. }
            | Action::DismissReplacedAgentOffer
            | Action::DismissOnboardingTip { .. }) => self.onboarding.dispatch(action),
            action @ (Action::UploadFile { .. }
            | Action::DownloadFile { .. }
            | Action::CancelTransfer { .. }
            | Action::DismissTransfer { .. }
            | Action::ResendFileLink { .. }
            | Action::DeleteFile { .. }
            | Action::SendFileToDevices { .. }
            | Action::GrantFileToRoom { .. }
            | Action::DismissFileProblem) => self.files.dispatch(action),
            action @ (Action::RefreshCoreUpgrade { .. }
            | Action::StartCoreUpgrade { .. }
            | Action::CancelCoreUpgrade
            | Action::SkipCoreUpgrade { .. }) => self.plugins.dispatch_core_upgrade(action),
            Action::SetAccentColor { color } => self.appearance.set_accent(color),
            Action::SetThemeMode { mode } => self.appearance.set_theme_mode(mode),
            Action::SetTextSize { size } => self.appearance.set_text_size(size),
            Action::SetFont { font } => self.appearance.set_font(font),
            Action::SelectTab { tab } => self.tabs.select(tab),
            Action::CloseTabOverlay => self.tabs.close_overlay(),
            Action::PinTab { tab, replacing } => self.tabs.pin(tab, replacing),
            Action::UnpinTab { tab } => self.tabs.unpin(tab),
            Action::MoveTab { tab, index } => self.tabs.move_tab(tab, index),
            Action::SetTabLabels { visible } => self.tabs.set_labels(visible),
            Action::SetTabDisplay { tab, display } => self.tabs.set_display(tab, display),
            Action::EnterFullscreen { tab } => self.tabs.enter_fullscreen(tab),
            Action::ExitFullscreen { tab } => self.tabs.exit_fullscreen(tab),
            Action::ReloadTab { tab } => self.tabs.reload(tab),
            Action::TabLoadFailed { tab, status } => self.tabs.load_failed(tab, status),
            Action::TabLoadSucceeded { tab } => self.tabs.load_succeeded(tab),
            Action::DismissUninstalledTabs => self.tabs.dismiss_uninstalled(),
            other => self.dispatch_plugins_or_chat(other),
        }
    }

    /// 当前的底栏和插件页面，用于首帧渲染。
    pub fn tabs(&self) -> TabsViewModel {
        self.tabs.view()
    }

    /// 订阅底栏，语义同 [`subscribe_session_list`](Self::subscribe_session_list)。
    pub fn subscribe_tabs(&self) -> Arc<TabsSubscription> {
        Arc::new(TabsSubscription::new(self.tabs.subscribe()))
    }

    /// 文档开始前、只在插件自己的主框架注入；原生通道名为 `wontonBridge`。
    pub fn tab_document_script(
        &self,
        tab: TabRef,
        environment: TabEnvironment,
    ) -> Result<String, CoreError> {
        bridge::document_script(&self.tabs, &self.appearance.view(), &tab, &environment)
    }

    /// 壳原样交入消息及原生回调的主框架标志、来源；核心校验来源，草稿不发送。
    pub fn tab_handle_message(
        &self,
        tab: TabRef,
        is_main_frame: bool,
        origin: String,
        message: String,
        environment: TabEnvironment,
    ) -> TabBridgeResult {
        bridge::handle_message(
            &self.tabs,
            &self.appearance,
            &tab,
            is_main_frame,
            &origin,
            &message,
            &environment,
        )
    }

    /// 外观、字号、布局或全屏状态变化后注入，不重新加载页面。
    pub fn tab_context_script(
        &self,
        tab: TabRef,
        environment: TabEnvironment,
    ) -> Result<String, CoreError> {
        bridge::context_script(&self.tabs, &self.appearance.view(), &tab, &environment)
    }

    pub fn tab_reselected_script(&self) -> String {
        bridge::reselected_script()
    }

    /// 连接变化时派发真实的核心状态；不把不确定状态当成已连接。
    pub fn tab_connection_script(&self) -> String {
        bridge::connection_script(&self.home.view().status)
    }

    /// 当前的外观（用户的设置和颜色令牌），用于首帧渲染。
    pub fn appearance(&self) -> AppearanceViewModel {
        self.appearance.view()
    }

    /// 订阅外观，语义同 [`subscribe_session_list`](Self::subscribe_session_list)。
    pub fn subscribe_appearance(&self) -> Arc<AppearanceSubscription> {
        Arc::new(AppearanceSubscription::new(self.appearance.subscribe()))
    }

    /// 配过的服务器，用于首帧渲染。
    pub fn servers(&self) -> ServersViewModel {
        self.servers.view()
    }

    /// 订阅配过的服务器，语义同 [`subscribe_session_list`](Self::subscribe_session_list)。
    pub fn subscribe_servers(&self) -> Arc<ServersSubscription> {
        Arc::new(ServersSubscription::new(self.servers.subscribe()))
    }

    /// 当前的设置首页，用于首帧渲染。
    pub fn settings(&self) -> SettingsViewModel {
        self.settings.view()
    }

    /// 订阅设置首页，语义同 [`subscribe_session_list`](Self::subscribe_session_list)。
    pub fn subscribe_settings(&self) -> Arc<SettingsSubscription> {
        Arc::new(SettingsSubscription::new(self.settings.subscribe()))
    }

    /// 当前的会话列表，用于首帧渲染。
    pub fn session_list(&self) -> SessionListViewModel {
        self.state.borrow().view_model()
    }

    /// 订阅会话列表。第一次 `next()` 立即返回当前值，之后每次状态变化返回最新值。
    pub fn subscribe_session_list(&self) -> Arc<SessionListSubscription> {
        Arc::new(SessionListSubscription::new(self.state.subscribe()))
    }

    /// 当前和家里服务器的连接，用于首帧渲染。
    pub fn home(&self) -> HomeViewModel {
        self.home.view()
    }

    /// 订阅和家里服务器的连接，语义同 [`subscribe_session_list`](Self::subscribe_session_list)。
    pub fn subscribe_home(&self) -> Arc<HomeSubscription> {
        Arc::new(HomeSubscription::new(self.home.subscribe()))
    }

    /// 当前的聊天状态（账号、同步、加密、房间列表），用于首帧渲染。
    pub fn chat(&self) -> ChatViewModel {
        self.chat.view()
    }

    /// 订阅聊天状态，语义同 [`subscribe_session_list`](Self::subscribe_session_list)。
    pub fn subscribe_chat(&self) -> Arc<ChatSubscription> {
        Arc::new(ChatSubscription::new(self.chat.subscribe()))
    }

    /// 发起私聊时能直接选的人：已装插件的 Bot 在前，然后是私聊过的人。`query` 不为空时按名字和 ID 筛。
    pub fn chat_contacts(&self, query: String) -> Vec<ChatContact> {
        let installed = self.plugins.view().installed;
        let plugins = installed
            .map(|installed| installed.plugins)
            .unwrap_or_default();
        self.chat.contacts(&query, &plugins)
    }

    /// 把发起私聊时输入的用户名或 Matrix ID 变成完整的 ID：只输用户名时补上本账号的服务器。不合法时返回
    /// `InvalidInput`，还没登录时返回 `NotReady`。
    pub fn resolve_user_id(&self, input: String) -> Result<String, CoreError> {
        self.chat.resolve_user_id(&input)
    }

    /// 订阅一个已加入房间的时间线：第一批是全部条目，之后是按顺序应用的增量。同一时间只打开一个房间，
    /// 打开另一个房间后前一个的订阅不再更新。还没登录、房间还没同步到本机时返回 `NotReady`。
    pub async fn subscribe_timeline(
        &self,
        room_id: String,
    ) -> Result<Arc<TimelineSubscription>, CoreError> {
        let chat = self.chat.clone();
        self.run(async move { chat.open_timeline(&room_id).await })
            .await
    }

    /// 取图片等媒体的原文件（时间线里给出的句柄）。核心负责下载、解密和缓存。
    pub async fn load_media(&self, media: MediaHandle) -> Result<Vec<u8>, CoreError> {
        let chat = self.chat.clone();
        self.run(async move { chat.load_media(media).await }).await
    }

    /// 当前的插件页面（已安装的插件、商店、安装的进展），用于首帧渲染。
    pub fn plugins(&self) -> PluginsViewModel {
        self.plugins.view()
    }

    /// 订阅插件页面，语义同 [`subscribe_session_list`](Self::subscribe_session_list)。
    pub fn subscribe_plugins(&self) -> Arc<PluginsSubscription> {
        Arc::new(PluginsSubscription::new(self.plugins.subscribe()))
    }
    /// 当前的新用户引导状态，用于首帧渲染。
    pub fn onboarding(&self) -> OnboardingViewModel {
        self.onboarding.view()
    }

    /// 订阅新用户引导，第一次 `next()` 立即返回当前值。
    pub fn subscribe_onboarding(&self) -> Arc<OnboardingSubscription> {
        Arc::new(OnboardingSubscription::new(self.onboarding.subscribe()))
    }

    /// 商店里一个插件的详情（来源最新的版本）。`source` 是 [`StoreItem::source`]。
    pub async fn store_plugin(&self, source: String) -> Result<PluginDetail, CoreError> {
        let plugins = self.plugins.clone();
        self.run(async move { plugins.store_plugin(&source).await })
            .await
    }

    /// 一个已安装插件的详情（装着的版本）。
    pub async fn installed_plugin(&self, id: String) -> Result<PluginDetail, CoreError> {
        let plugins = self.plugins.clone();
        self.run(async move { plugins.installed_plugin(&id).await })
            .await
    }

    /// 安装前的预览：家里的服务器解析来源、取插件包，给出要主人确认的权限和说明，确认页渲染它。`source` 是
    /// [`StoreItem::source`]，或者主人输入的 `github:<owner>/<repo>[@<tag>]`。
    pub async fn preview_install(&self, source: String) -> Result<InstallPreview, CoreError> {
        let plugins = self.plugins.clone();
        self.run(async move { plugins.preview_install(source).await })
            .await
    }

    /// 取家里服务器上的资料（ADR-0030），设置页和填资料的页面渲染它。
    pub async fn owner_profile(&self) -> Result<OwnerProfile, CoreError> {
        let plugins = self.plugins.clone();
        self.run(async move { plugins.owner_profile().await }).await
    }

    /// 保存资料，名字同步成 Matrix 上的显示名。不合规则时返回 `InvalidInput`，原因由家里的服务器写好。保存后要重新取安装页的预览。
    pub async fn save_owner_profile(
        &self,
        update: OwnerProfileUpdate,
    ) -> Result<OwnerProfile, CoreError> {
        let plugins = self.plugins.clone();
        let saved = self
            .run(async move { plugins.save_owner_profile(update).await })
            .await?;
        if let Some(name) = saved.display_name.clone() {
            self.chat.set_owner_display_name(name);
        }
        Ok(saved)
    }

    /// 升级前的预览：升到已安装列表里提示的版本。
    pub async fn preview_upgrade(&self, id: String) -> Result<InstallPreview, CoreError> {
        let plugins = self.plugins.clone();
        self.run(async move { plugins.preview_upgrade(&id).await })
            .await
    }

    /// 插件的压缩包最大多少字节。壳读文件之前先看大小，太大的不读进内存。
    pub fn max_plugin_archive_bytes(&self) -> u64 {
        wonton_protocol::MAX_PLUGIN_ARCHIVE_BYTES
    }

    /// 主人选的压缩包的预览（ADR-0031）：`archive` 是文件的内容，壳只负责选文件和读字节。确认后按预览安装，不用再传。
    pub async fn preview_upload(&self, archive: Vec<u8>) -> Result<InstallPreview, CoreError> {
        let plugins = self.plugins.clone();
        self.run(async move { plugins.preview_upload(archive).await })
            .await
    }

    /// 换版本的预览：换到主人选的来源，可以比装着的旧，确认页上会醒目提示。
    pub async fn preview_upgrade_from(
        &self,
        id: String,
        source: String,
    ) -> Result<InstallPreview, CoreError> {
        let plugins = self.plugins.clone();
        self.run(async move { plugins.preview_upgrade_from(&id, source).await })
            .await
    }

    /// 换版本的预览：换到主人选的压缩包。
    pub async fn preview_upgrade_upload(
        &self,
        id: String,
        archive: Vec<u8>,
    ) -> Result<InstallPreview, CoreError> {
        let plugins = self.plugins.clone();
        self.run(async move { plugins.preview_upgrade_upload(&id, archive).await })
            .await
    }

    /// 添加来源：地址可以是仓库地址、`owner/repo`，或者索引的 https 地址。
    pub async fn add_store_source(&self, address: String) -> Result<AddedSource, CoreError> {
        let plugins = self.plugins.clone();
        self.run(async move { plugins.add_store_source(address).await })
            .await
    }

    /// 删除添加的来源，已安装的插件不动。
    pub async fn remove_store_source(&self, address: String) -> Result<(), CoreError> {
        let plugins = self.plugins.clone();
        self.run(async move { plugins.remove_store_source(&address).await })
            .await
    }

    /// 开关插件网页的局域网端口（ADR-0023）。核心要重建容器，做完才回应；返回开关之后的详情。
    pub async fn set_lan_webui(
        &self,
        id: String,
        enabled: bool,
    ) -> Result<PluginDetail, CoreError> {
        let plugins = self.plugins.clone();
        self.run(async move { plugins.set_lan_webui(&id, enabled).await })
            .await
    }

    /// 开关“每次都问”（ADR-0033）：打开后这个 Agent 每次更新都发卡片；返回开关之后的详情。
    pub async fn set_ask_every_update(
        &self,
        id: String,
        enabled: bool,
    ) -> Result<PluginDetail, CoreError> {
        let plugins = self.plugins.clone();
        self.run(async move { plugins.set_ask_every_update(&id, enabled).await })
            .await
    }

    /// 撤销 Bot 往这个路径存文件的授权（ADR-0027），只拦后面的调用；返回撤销之后的详情。
    pub async fn revoke_save_grant(
        &self,
        id: String,
        folder: String,
    ) -> Result<PluginDetail, CoreError> {
        let plugins = self.plugins.clone();
        self.run(async move { plugins.revoke_save_grant(&id, &folder).await })
            .await
    }

    /// 取插件的图标或截图：PNG 或 JPEG 的字节，经隧道取回并缓存，同 [`load_media`](Self::load_media)。
    pub async fn load_plugin_image(&self, image: PluginImage) -> Result<Vec<u8>, CoreError> {
        let plugins = self.plugins.clone();
        self.run(async move { plugins.load_image(&image).await })
            .await
    }

    /// 给 [`UploadSource::Opaque`](wonton_protocol::UploadSource::Opaque) 的来源换文件描述符。壳在创建核心之后、发出
    /// 第一个操作之前设置：App 重启后核心要接着传。
    pub fn set_file_opener(&self, opener: Arc<dyn FileOpener>) {
        self.files.set_opener(opener);
    }

    /// 当前的传输列表，用于首帧渲染。
    pub fn transfers(&self) -> TransfersViewModel {
        self.files.view()
    }

    /// 这个房间的传输条列什么。不走网络，传输列表一变就可以重新取。
    pub fn room_transfers(&self, room_id: String) -> RoomTransfers {
        self.files
            .view()
            .for_room(&room_id, &self.chat.view().rooms)
    }

    /// 订阅传输列表，语义同 [`subscribe_session_list`](Self::subscribe_session_list)。
    pub fn subscribe_transfers(&self) -> Arc<TransfersSubscription> {
        Arc::new(TransfersSubscription::new(self.files.subscribe()))
    }

    /// 消息下面的文件卡片：取信息并缓存。结果写在卡片里，壳等待时画 [`FileCard::Loading`]。
    pub async fn file_card(&self, id: FileId) -> FileCard {
        let files = self.files.clone();
        let card = self.run(async move { Ok(files.card(id).await) }).await;
        card.unwrap_or_else(|error| FileCard::Failed {
            reason: FileFailure::Other,
            message: error.to_string(),
        })
    }

    /// 卡片上的按钮和下载进展。不走网络，传输列表一变就可以重新取。
    pub fn file_card_actions(&self, info: FileInfo) -> FileCardActions {
        self.files.card_actions(&info)
    }

    /// 文件的缩略图：JPEG 字节。不在访问范围里时返回 `Unauthorized`；文件没了、这个类型没有缩略图时返回 `InvalidInput`。
    pub async fn file_thumbnail(&self, id: FileId) -> Result<Vec<u8>, CoreError> {
        let files = self.files.clone();
        self.run(async move { files.thumbnail(id).await }).await
    }

    /// 给 App 自己的播放器用的回环地址，带 `Range`：播放器不能给单个请求设代理。令牌每次启动换一个；App 回到前台后要重新取，
    /// 端口可能变了。
    pub async fn file_stream_url(&self, id: FileId) -> Result<String, CoreError> {
        let files = self.files.clone();
        self.run(async move { files.stream_url(id).await }).await
    }

    /// 播放器放不了时问为什么：连不上家、家里的服务器报错，还是这台手机不支持这个格式。说明里写着可以下载后用其他 App 打开。
    pub async fn file_playback_problem(&self, id: FileId) -> FileProblem {
        let files = self.files.clone();
        let asked = id.clone();
        let problem = self
            .run(async move { Ok(files.playback_problem(asked).await) })
            .await;
        problem.unwrap_or_else(|error| FileProblem {
            operation: FileOperation::Play,
            file: id,
            reason: FileFailure::Other,
            message: error.to_string(),
        })
    }

    /// 局域网设置：插件的局域网端口开在哪些地址上、可以选的网卡，以及开了端口的插件。
    pub async fn lan_settings(&self) -> Result<LanSettings, CoreError> {
        Err(lan_unsupported())
    }

    /// 改选局域网端口开在哪里。家里的服务器按新地址重建开了端口的插件容器，做完后返回新的设置；App 断开了它也
    /// 照样做完。选择不合法（一项都没选、网卡或地址不在宿主上、不是私有地址、[`LanChoice::Unknown`]），或者地址由
    /// `WONTON_LAN_ADDRESSES` 指定时返回 `InvalidInput`；选的网卡上没有私有 IPv4 地址、家里的服务器检测不了网卡时
    /// 返回 `NotReady`。
    pub async fn set_lan_choice(&self, choice: LanChoice) -> Result<LanSettings, CoreError> {
        let _ = choice;
        Err(lan_unsupported())
    }

    /// 在最近取到的商店列表里按关键词找插件：名字、说明、标签和 ID，不分大小写，官方的在前。还没取过商店时为空。
    pub fn search_store(&self, query: String) -> Vec<StoreItem> {
        self.plugins.search_store(&query)
    }

    /// 处理收到的一条推送：`payload` 是壳在本机解开后的明文（Android 上 UnifiedPush 连接库已经按 RFC 8291 解开）。
    /// 核心经隧道取回那条消息、在本机解密，给出要弹出的通知；取不到正文时通知里照实写原因。推送唤醒的进程可以
    /// 一创建核心就调用，核心等会话从本机恢复。不用显示（按推送规则不该响、消息已撤回）时返回空，内容不是家里
    /// 推送代理发的格式时返回 `InvalidInput`。最多等二三十秒。
    pub async fn resolve_push(
        &self,
        payload: Vec<u8>,
    ) -> Result<Option<PushNotification>, CoreError> {
        let push = self.push.clone();
        self.run(async move { push.resolve(&payload).await }).await
    }
}

impl ClientCore {
    /// 关掉隧道的定时检查，只留回到前台、网络变化和重试触发的检查。只给测试用。
    #[doc(hidden)]
    pub fn stop_periodic_checks(&self) {
        self.home.stop_periodic_checks();
    }

    /// 指定 iroh 的网络配置。[`Network::DirectOnly`] 不连中继、不做地址发现，本机和局域网测试用。
    pub fn with_network(data_dir: &str, network: Network) -> Result<Arc<Self>, CoreError> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("wonton-core")
            .enable_all()
            .build()
            .expect("创建 tokio 运行时失败");
        let storage = Storage::open(data_dir)?;
        let home = HomeLink::new(runtime.handle().clone(), storage, network)?;
        let (card_success, card_refreshes) = tokio::sync::mpsc::unbounded_channel::<String>();
        let chat = ChatLink::new(
            runtime.handle().clone(),
            home.clone(),
            Path::new(data_dir),
            card_success,
        )?;
        let push = PushLink::new(runtime.handle().clone(), home.clone(), chat.clone());
        let plugins = PluginsLink::new(runtime.handle().clone(), home.clone(), Path::new(data_dir));
        runtime.spawn(follow_card_success(plugins.clone(), card_refreshes));
        let tabs = TabsLink::new(
            runtime.handle().clone(),
            home.clone(),
            plugins.clone(),
            Path::new(data_dir),
        );
        let files = FilesLink::new(
            runtime.handle().clone(),
            home.clone(),
            chat.clone(),
            Path::new(data_dir),
        );
        let appearance = AppearanceLink::new(Path::new(data_dir));
        let onboarding = OnboardingLink::new(
            runtime.handle().clone(),
            home.clone(),
            chat.clone(),
            plugins.clone(),
            Path::new(data_dir),
        );
        let servers = ServersLink::new(runtime.handle(), home.subscribe());
        let settings = SettingsLink::new(
            runtime.handle(),
            home.subscribe(),
            chat.subscribe(),
            plugins.subscribe(),
        );
        Ok(Arc::new(Self {
            state: watch::Sender::new(SessionStore::default()),
            home,
            chat,
            push,
            plugins,
            tabs,
            files,
            appearance,
            onboarding,
            settings,
            servers,
            runtime: Some(runtime),
        }))
    }

    /// 在核心自己的运行时里执行：壳（Swift / Kotlin）驱动导出的 async 方法时，线程上没有 tokio 运行时，
    /// matrix-sdk 的网络操作离不开它。
    async fn run<T: Send + 'static>(
        &self,
        task: impl Future<Output = Result<T, CoreError>> + Send + 'static,
    ) -> Result<T, CoreError> {
        let runtime = self.runtime.as_ref().expect("核心释放前运行时一直在");
        runtime
            .spawn(task)
            .await
            .map_err(|error| CoreError::Upstream {
                reason: format!("核心的后台任务异常结束：{error}"),
            })?
    }

    /// `dispatch` 的后半：插件的操作，其余都是聊天的。分开放是因为函数长度有上限。
    fn dispatch_plugins_or_chat(&self, action: Action) -> Result<(), CoreError> {
        match action {
            Action::SetLanguages { languages } => {
                self.plugins.set_languages(languages.clone());
                self.onboarding.set_languages(languages);
                Ok(())
            }
            Action::RefreshPlugins => {
                self.plugins.refresh_installed();
                Ok(())
            }
            Action::StartPlugin { plugin_id } => self.plugins.start(plugin_id),
            Action::RefreshStore { force } => {
                self.plugins.refresh_store(force);
                Ok(())
            }
            Action::InstallPlugin { preview } => self.plugins.install(preview),
            Action::UpgradePlugin { preview } => self.plugins.upgrade(preview),
            Action::UninstallPlugin { id, delete_data } => self.plugins.uninstall(id, delete_data),
            Action::DismissPluginTask { id } => self.plugins.dismiss_task(&id),
            Action::StartTakeover { id } => {
                self.plugins.start_takeover(id);
                Ok(())
            }
            Action::EndTakeover { id } => {
                self.plugins.end_takeover(&id);
                Ok(())
            }
            Action::RefreshRegistryMirror => {
                self.plugins.refresh_registry_mirror();
                Ok(())
            }
            Action::SaveRegistryMirror { mirror } => self.plugins.save_registry_mirror(mirror),
            Action::TestRegistryMirrors => self.plugins.test_registry_mirrors(),
            Action::RefreshEgressProxy => {
                self.plugins.refresh_egress();
                Ok(())
            }
            Action::SaveEgressProxy {
                address,
                direct_cidrs,
                use_plugin,
            } => self
                .plugins
                .save_egress(&address, &direct_cidrs, use_plugin),
            Action::ConfirmEgressProxy => self.plugins.confirm_egress(),
            Action::DismissEgressConfirmation => {
                self.plugins.dismiss_egress_confirmation();
                Ok(())
            }
            Action::AskAgent { text } => {
                let bot = self.onboarding.view().primary_bot_user_id.ok_or_else(|| {
                    CoreError::NotReady {
                        reason: "还没有默认 Agent：先在设置里选一个".to_owned(),
                    }
                })?;
                self.chat.ask(&bot, &text)
            }
            chat => self.chat.dispatch(chat),
        }
    }

    fn update_sessions(
        &self,
        change: impl FnOnce(&mut SessionStore) -> Result<(), CoreError>,
    ) -> Result<(), CoreError> {
        let mut outcome = Ok(());
        self.state.send_if_modified(|store| match change(store) {
            Ok(()) => true,
            Err(error) => {
                outcome = Err(error);
                false
            }
        });
        outcome
    }
}

/// 局域网设置的接口先于实现合并（ADR-0019）：实现之前如实失败。
fn lan_unsupported() -> CoreError {
    CoreError::NotReady {
        reason: "这个版本的客户端核心不支持局域网设置".into(),
    }
}

impl Drop for ClientCore {
    fn drop(&mut self) {
        self.files.close();
        // 不等后台任务结束：壳可能在主线程上释放核心，测试可能在异步上下文里释放。
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_background();
        }
    }
}
