use std::path::PathBuf;
#[cfg(feature = "ui-fake")]
use std::sync::RwLockReadGuard;
#[cfg(feature = "ui-fake")]
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, RwLock};

use tokio::sync::watch;
use wonton_l10n::l10n;
use wonton_protocol::{
    Action, AddedSource, AppearanceViewModel, ChatContact, ChatViewModel, ComposerDraft, CoreError,
    FileCard, FileCardActions, FileFailure, FileId, FileInfo, FileOperation, FileProblem,
    FriendsViewModel, HomeViewModel, InstallPreview, LanChoice, LanSettings, Me, MediaHandle,
    OnboardingViewModel, OwnerProfileUpdate, PeersViewModel, PluginDetail, PluginImage,
    PluginsViewModel, RoomTransfers, SecretString, ServersViewModel, SessionListViewModel,
    SettingsViewModel, StoreItem, TabRef, TabsViewModel, TimelineTarget, TransfersViewModel,
};

use crate::appearance::AppearanceLink;
use crate::chat::TimelineSubscription;
use crate::files::FileOpener;
use crate::home::Network;
use crate::l10n::{CATALOG, language};
use crate::scope::{Scope, Views, Visits};
use crate::servers::ServerList;
use crate::session_store::SessionStore;
use crate::subscription::{
    AppearanceSubscription, ChatSubscription, FriendsSubscription, HomeSubscription,
    OnboardingSubscription, PeersSubscription, PluginsSubscription, ServersSubscription,
    SessionListSubscription, SettingsSubscription, TabsSubscription, TransfersSubscription,
};
use switch::Replay;

mod bot_contacts;
mod browser;
mod composer;
mod dispatch;
mod limits;
mod media;
mod push_route;
mod settings;
mod sharing;
mod switch;
mod tabs;
#[cfg(feature = "test-hooks")]
mod test_hooks;
mod test_support;
mod tool_policy;
#[cfg(feature = "ui-fake")]
mod ui_fake;

/// 客户端核心的入口。原生壳在启动时创建一个实例并一直持有。
///
/// 状态只存在这里：watch 通道既保存当前状态，也负责通知订阅者。连着服务器的部件都在当前的作用域里
/// （[`Scope`]），网络操作在作用域自己的 tokio 运行时里执行，进度和结果体现在 ViewModel 里。切换服务器时
/// 换掉作用域，壳的订阅接在 `views` 上，不用重建。
#[derive(uniffi::Object)]
pub struct ClientCore {
    state: watch::Sender<SessionStore>,
    appearance: AppearanceLink,
    servers: Arc<ServerList>,
    root: PathBuf,
    network: Network,
    scope: RwLock<Arc<Scope>>,
    /// 壳订阅的 ViewModel 发送端。有假数据入口时整组换成独立的发送端，包一层锁；没有这个 feature 时保持
    /// 直接字段，生产构建不多一次读锁。
    #[cfg(feature = "ui-fake")]
    views: RwLock<Views>,
    #[cfg(not(feature = "ui-fake"))]
    views: Views,
    /// 切换后要在新作用域里重放的、壳只发一次的输入。
    replay: Mutex<Replay>,
    /// 串行化切换：列表改动和换作用域要一起做完。
    switching: Arc<Mutex<()>>,
    /// 为不是当前这台的服务器临时开的作用域。
    visits: Arc<Visits>,
    /// 正在给哪台后台登记哪个推送地址，`push_route` 拿它和落盘记录一起去重。
    in_flight_registrations: push_route::InFlightRegistrations,
    matrix_login: Arc<crate::matrix_login::MatrixLogins>,
    /// 测试用的假数据入口（`new_ui_fake`）开着：操作不落到作用域，ViewModel 保持塞进去的假数据。
    #[cfg(feature = "ui-fake")]
    ui_fake: AtomicBool,
}

#[uniffi::export]
impl ClientCore {
    /// `data_dir` 是 App 的私有数据目录（iOS 的 Application Support、Android 的 `filesDir`），
    /// 存设备密钥和上次的连接信息。上次连过家里的服务器时，创建后自动重连。
    #[uniffi::constructor]
    pub fn new(data_dir: String) -> Result<Arc<Self>, CoreError> {
        Self::with_network(&data_dir, Network::Internet)
    }

    /// 当前的底栏和插件页面，用于首帧渲染。
    pub fn tabs(&self) -> TabsViewModel {
        self.views().tabs.borrow().clone()
    }

    /// 订阅底栏，语义同 [`subscribe_session_list`](Self::subscribe_session_list)。
    pub fn subscribe_tabs(&self) -> Arc<TabsSubscription> {
        Arc::new(TabsSubscription::new(self.views().tabs.subscribe()))
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
        self.views().settings.borrow().clone()
    }

    /// 订阅设置首页，语义同 [`subscribe_session_list`](Self::subscribe_session_list)。
    pub fn subscribe_settings(&self) -> Arc<SettingsSubscription> {
        Arc::new(SettingsSubscription::new(self.views().settings.subscribe()))
    }

    /// 当前的“其他服务器”页面（ADR-0050），用于首帧渲染。
    pub fn peers(&self) -> PeersViewModel {
        self.views().peers.borrow().clone()
    }

    /// 订阅“其他服务器”页面，语义同 [`subscribe_session_list`](Self::subscribe_session_list)。
    pub fn subscribe_peers(&self) -> Arc<PeersSubscription> {
        Arc::new(PeersSubscription::new(self.views().peers.subscribe()))
    }

    /// 当前的好友页（ADR-0051），用于首帧渲染。
    pub fn friends(&self) -> FriendsViewModel {
        self.views().friends.borrow().clone()
    }

    /// 订阅好友页，语义同 [`subscribe_session_list`](Self::subscribe_session_list)。
    pub fn subscribe_friends(&self) -> Arc<FriendsSubscription> {
        Arc::new(FriendsSubscription::new(self.views().friends.subscribe()))
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
        self.views().home.borrow().clone()
    }

    /// 订阅和家里服务器的连接，语义同 [`subscribe_session_list`](Self::subscribe_session_list)。
    pub fn subscribe_home(&self) -> Arc<HomeSubscription> {
        Arc::new(HomeSubscription::new(self.views().home.subscribe()))
    }

    /// 当前的聊天状态（账号、同步、加密、房间列表），用于首帧渲染。
    pub fn chat(&self) -> ChatViewModel {
        self.views().chat.borrow().clone()
    }

    /// 订阅聊天状态，语义同 [`subscribe_session_list`](Self::subscribe_session_list)。
    pub fn subscribe_chat(&self) -> Arc<ChatSubscription> {
        Arc::new(ChatSubscription::new(self.views().chat.subscribe()))
    }

    /// 发起私聊时能直接选的人：已装插件的 Bot 在前，然后是私聊过的人。`query` 不为空时按名字和 ID 筛。
    pub fn chat_contacts(&self, query: String) -> Vec<ChatContact> {
        let installed = self.scope().plugins.view().installed;
        let plugins = installed
            .map(|installed| installed.plugins)
            .unwrap_or_default();
        self.scope().chat.contacts(&query, &plugins)
    }

    /// 把发起私聊时输入的用户名或 Matrix ID 变成完整的 ID：只输用户名时补上本账号的服务器。不合法时返回
    /// `InvalidInput`，还没登录时返回 `NotReady`。
    pub fn resolve_user_id(&self, input: String) -> Result<String, CoreError> {
        self.scope().chat.resolve_user_id(&input)
    }

    /// 订阅一条时间线（主时间线或旁聊，ADR-0093 §5）：第一批是全部条目，之后是按顺序应用的增量。
    /// 同一时间只打开一个房间，打开另一个房间后前一个的订阅不再更新。还没登录、房间还没同步到本机时
    /// 返回 `NotReady`；旁聊时间线还没实现，`thread_root` 不为空时返回 `NotReady`。
    pub async fn subscribe_timeline(
        &self,
        target: TimelineTarget,
    ) -> Result<Arc<TimelineSubscription>, CoreError> {
        let room_id = self.chat_timeline_target(&target)?;
        let scope = self.scope();
        let chat = scope.chat.clone();
        let room_view = scope.tabs.subscribe_room_view();
        scope
            .run(async move { chat.open_timeline(&room_id, room_view).await })
            .await
    }

    /// 一条时间线（主时间线或旁聊）输入框里的草稿（正文和正在回复的消息），没打过字也没在回复时是
    /// `None`。换房间、重启核心都还在，主人回来接着写。旁聊时间线还没实现，`thread_root` 不为空时
    /// 返回 `NotReady`（ADR-0093 §5）。
    pub async fn draft(&self, target: TimelineTarget) -> Result<Option<ComposerDraft>, CoreError> {
        let room_id = self.chat_timeline_target(&target)?;
        let scope = self.scope();
        let chat = scope.chat.clone();
        scope.run(async move { chat.draft(&room_id).await }).await
    }

    /// 开旁聊（ADR-0093 §4）：发根消息（带 `app.wonton.side_chat` 的 `m.notice`），等服务器回事件 ID，
    /// 把第一句话放进这个话题的发送队列，返回根的事件 ID。根没发出去时返回错误，不留下空旁聊。
    /// 旁聊还没实现（接口先合并），现在返回 `NotReady`。
    pub async fn start_side_chat(
        &self,
        room_id: String,
        body: String,
    ) -> Result<String, CoreError> {
        let _ = (room_id, body);
        Err(self.chat_side_chat_not_ready())
    }

    /// 拆 [`TimelineTarget`]：主时间线回房间 ID；带话题根的旁聊时间线还没实现，报“还不能用”。
    fn chat_timeline_target(&self, target: &TimelineTarget) -> Result<String, CoreError> {
        if target.thread_root.is_some() {
            return Err(self.chat_side_chat_not_ready());
        }
        Ok(target.room_id.clone())
    }

    fn chat_side_chat_not_ready(&self) -> CoreError {
        CoreError::NotReady {
            reason: l10n!(CATALOG, language(), "side-chat-not-ready"),
        }
    }

    /// 取图片等媒体的原文件（时间线里给出的句柄）。核心负责下载、解密和缓存。
    pub async fn load_media(&self, media: MediaHandle) -> Result<Vec<u8>, CoreError> {
        let scope = self.scope();
        let chat = scope.chat.clone();
        scope.run(async move { chat.load_media(media).await }).await
    }

    /// 视频头像的本机缓存文件；相同媒体的并发请求共用一次下载。
    pub async fn avatar_file(&self, media: MediaHandle) -> Result<String, CoreError> {
        let scope = self.scope();
        let chat = scope.chat.clone();
        scope
            .run(async move { chat.avatar_file(media).await })
            .await
    }

    /// 当前的插件页面（已安装的插件、商店、安装的进展），用于首帧渲染。
    pub fn plugins(&self) -> PluginsViewModel {
        self.views().plugins.borrow().clone()
    }

    /// 订阅插件页面，语义同 [`subscribe_session_list`](Self::subscribe_session_list)。
    pub fn subscribe_plugins(&self) -> Arc<PluginsSubscription> {
        Arc::new(PluginsSubscription::new(self.views().plugins.subscribe()))
    }
    /// 当前的新用户引导状态，用于首帧渲染。
    pub fn onboarding(&self) -> OnboardingViewModel {
        self.views().onboarding.borrow().clone()
    }

    /// 订阅新用户引导，第一次 `next()` 立即返回当前值。
    pub fn subscribe_onboarding(&self) -> Arc<OnboardingSubscription> {
        Arc::new(OnboardingSubscription::new(
            self.views().onboarding.subscribe(),
        ))
    }

    /// 商店里一个插件的详情（来源最新的版本）。`source` 是 [`StoreItem::source`]。
    pub async fn store_plugin(&self, source: String) -> Result<PluginDetail, CoreError> {
        let scope = self.scope();
        let plugins = scope.plugins.clone();
        scope
            .run(async move { plugins.store_plugin(&source).await })
            .await
    }

    /// 一个已安装插件的详情（装着的版本）。
    pub async fn installed_plugin(&self, id: String) -> Result<PluginDetail, CoreError> {
        let scope = self.scope();
        let plugins = scope.plugins.clone();
        scope
            .run(async move { plugins.installed_plugin(&id).await })
            .await
    }

    /// 安装前的预览：家里的服务器解析来源、取插件包，给出要主人确认的权限和说明，确认页渲染它。`source` 是
    /// [`StoreItem::source`]，或者主人输入的 `github:<owner>/<repo>[@<tag>]`。
    pub async fn preview_install(&self, source: String) -> Result<InstallPreview, CoreError> {
        let scope = self.scope();
        let plugins = scope.plugins.clone();
        scope
            .run(async move { plugins.preview_install(source).await })
            .await
    }

    /// 取家里服务器上这台设备的人（ADR-0045），设置页和填资料的页面渲染它。
    pub async fn me(&self) -> Result<Me, CoreError> {
        let scope = self.scope();
        let plugins = scope.plugins.clone();
        scope.run(async move { plugins.me().await }).await
    }

    /// 保存资料（主人写 owner.json，家人写自己那条），名字同步成 Matrix 上的显示名。不合规则时返回
    /// `InvalidInput`，原因由家里的服务器写好。保存后要重新取安装页的预览。
    pub async fn save_me(&self, update: OwnerProfileUpdate) -> Result<Me, CoreError> {
        let scope = self.scope();
        let plugins = scope.plugins.clone();
        let saved = scope
            .run(async move { plugins.save_profile(update).await })
            .await?;
        if let Some(name) = saved.display_name.clone() {
            scope.chat.set_owner_display_name(name);
        }
        Ok(saved)
    }

    /// 升级前的预览：升到已安装列表里提示的版本。
    pub async fn preview_upgrade(&self, id: String) -> Result<InstallPreview, CoreError> {
        let scope = self.scope();
        let plugins = scope.plugins.clone();
        scope
            .run(async move { plugins.preview_upgrade(&id).await })
            .await
    }

    /// 主人选的压缩包的预览（ADR-0031）：`archive` 是文件的内容，壳只负责选文件和读字节。确认后按预览安装，不用再传。
    pub async fn preview_upload(&self, archive: Vec<u8>) -> Result<InstallPreview, CoreError> {
        let scope = self.scope();
        let plugins = scope.plugins.clone();
        scope
            .run(async move { plugins.preview_upload(archive).await })
            .await
    }

    /// 换版本的预览：换到主人选的来源，可以比装着的旧，确认页上会醒目提示。
    pub async fn preview_upgrade_from(
        &self,
        id: String,
        source: String,
    ) -> Result<InstallPreview, CoreError> {
        let scope = self.scope();
        let plugins = scope.plugins.clone();
        scope
            .run(async move { plugins.preview_upgrade_from(&id, source).await })
            .await
    }

    /// 换版本的预览：换到主人选的压缩包。
    pub async fn preview_upgrade_upload(
        &self,
        id: String,
        archive: Vec<u8>,
    ) -> Result<InstallPreview, CoreError> {
        let scope = self.scope();
        let plugins = scope.plugins.clone();
        scope
            .run(async move { plugins.preview_upgrade_upload(&id, archive).await })
            .await
    }

    /// 添加来源：地址可以是仓库地址、`owner/repo`，或者索引的 https 地址。
    pub async fn add_store_source(&self, address: String) -> Result<AddedSource, CoreError> {
        let scope = self.scope();
        let plugins = scope.plugins.clone();
        scope
            .run(async move { plugins.add_store_source(address).await })
            .await
    }

    /// 删除添加的来源，已安装的插件不动。
    pub async fn remove_store_source(&self, address: String) -> Result<(), CoreError> {
        let scope = self.scope();
        let plugins = scope.plugins.clone();
        scope
            .run(async move { plugins.remove_store_source(&address).await })
            .await
    }

    /// 开关插件网页的局域网端口（ADR-0023）。核心要重建容器，做完才回应；返回开关之后的详情。
    pub async fn set_lan_webui(
        &self,
        id: String,
        enabled: bool,
    ) -> Result<PluginDetail, CoreError> {
        let scope = self.scope();
        let plugins = scope.plugins.clone();
        scope
            .run(async move { plugins.set_lan_webui(&id, enabled).await })
            .await
    }

    /// 开关插件的一项可选权限（ADR-0057）：打开时核心重建拿到字段的服务，做完才回应；返回开关之后的详情。
    pub async fn set_plugin_permission(
        &self,
        id: String,
        permission: String,
        enabled: bool,
    ) -> Result<PluginDetail, CoreError> {
        let scope = self.scope();
        let plugins = scope.plugins.clone();
        scope
            .run(async move {
                plugins
                    .set_plugin_permission(&id, &permission, enabled)
                    .await
            })
            .await
    }

    /// 填一项插件密钥或清空可选项（ADR-0064）：核心重建用到它的服务，做完才回应；返回之后的详情。
    pub async fn set_plugin_secret(
        &self,
        id: String,
        name: String,
        value: Option<SecretString>,
    ) -> Result<PluginDetail, CoreError> {
        let scope = self.scope();
        let plugins = scope.plugins.clone();
        scope
            .run(async move { plugins.set_secret_now(id, name, value).await })
            .await
    }

    /// 开关“每次都问”（ADR-0033）：打开后这个 Agent 每次更新都发卡片；返回开关之后的详情。
    pub async fn set_ask_every_update(
        &self,
        id: String,
        enabled: bool,
    ) -> Result<PluginDetail, CoreError> {
        let scope = self.scope();
        let plugins = scope.plugins.clone();
        scope
            .run(async move { plugins.set_ask_every_update(&id, enabled).await })
            .await
    }

    /// 撤销 Bot 往这个路径存文件的授权（ADR-0027），只拦后面的调用；返回撤销之后的详情。
    pub async fn revoke_save_grant(
        &self,
        id: String,
        folder: String,
    ) -> Result<PluginDetail, CoreError> {
        let scope = self.scope();
        let plugins = scope.plugins.clone();
        scope
            .run(async move { plugins.revoke_save_grant(&id, &folder).await })
            .await
    }

    /// 取插件的图标或截图：PNG 或 JPEG 的字节，经隧道取回并缓存，同 [`load_media`](Self::load_media)。
    pub async fn load_plugin_image(&self, image: PluginImage) -> Result<Vec<u8>, CoreError> {
        let scope = self.scope();
        let plugins = scope.plugins.clone();
        scope
            .run(async move { plugins.load_image(&image).await })
            .await
    }

    /// 给 [`UploadSource::Opaque`](wonton_protocol::UploadSource::Opaque) 的来源换文件描述符。壳在创建核心之后、发出
    /// 第一个操作之前设置：App 重启后核心要接着传。
    pub fn set_file_opener(&self, opener: Arc<dyn FileOpener>) {
        self.scope().files.set_opener(opener.clone());
        self.replay().opener = Some(opener);
    }

    /// 当前的传输列表，用于首帧渲染。
    pub fn transfers(&self) -> TransfersViewModel {
        self.views().transfers.borrow().clone()
    }

    /// 这个房间的传输条列什么。不走网络，传输列表一变就可以重新取。
    pub fn room_transfers(&self, room_id: String) -> RoomTransfers {
        let scope = self.scope();
        scope
            .files
            .view()
            .for_room(&room_id, &scope.chat.view().rooms)
    }

    /// 订阅传输列表，语义同 [`subscribe_session_list`](Self::subscribe_session_list)。
    pub fn subscribe_transfers(&self) -> Arc<TransfersSubscription> {
        Arc::new(TransfersSubscription::new(
            self.views().transfers.subscribe(),
        ))
    }

    /// 消息下面的文件卡片：取信息并缓存。结果写在卡片里，壳等待时画 [`FileCard::Loading`]。
    pub async fn file_card(&self, id: FileId) -> FileCard {
        let scope = self.scope();
        let files = scope.files.clone();
        let card = scope.run(async move { Ok(files.card(id).await) }).await;
        card.unwrap_or_else(|error| FileCard::Failed {
            reason: FileFailure::Other,
            message: error.to_string(),
        })
    }

    /// 卡片上的按钮和下载进展。不走网络，传输列表一变就可以重新取。
    pub fn file_card_actions(&self, info: FileInfo) -> FileCardActions {
        self.scope().files.card_actions(&info)
    }

    /// 文件的缩略图：JPEG 字节。不在访问范围里时返回 `Unauthorized`；文件没了、这个类型没有缩略图时返回 `InvalidInput`。
    pub async fn file_thumbnail(&self, id: FileId) -> Result<Vec<u8>, CoreError> {
        let scope = self.scope();
        let files = scope.files.clone();
        scope.run(async move { files.thumbnail(id).await }).await
    }

    /// 给 App 自己的播放器用的回环地址，带 `Range`：播放器不能给单个请求设代理。令牌每次启动换一个；App 回到前台后要重新取，
    /// 端口可能变了。
    pub async fn file_stream_url(&self, id: FileId) -> Result<String, CoreError> {
        let scope = self.scope();
        let files = scope.files.clone();
        scope.run(async move { files.stream_url(id).await }).await
    }

    /// 播放器放不了时问为什么：连不上家、家里的服务器报错，还是这台手机不支持这个格式。只有格式放不了时，
    /// 说明里才提可以下载后用其他 App 打开。
    pub async fn file_playback_problem(&self, id: FileId) -> FileProblem {
        let scope = self.scope();
        let files = scope.files.clone();
        let asked = id.clone();
        let problem = scope
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
        self.scope().plugins.search_store(&query)
    }
}

impl ClientCore {
    #[cfg(feature = "ui-fake")]
    pub(crate) fn views(&self) -> RwLockReadGuard<'_, Views> {
        self.views
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    #[cfg(not(feature = "ui-fake"))]
    pub(crate) fn views(&self) -> &Views {
        &self.views
    }

    /// 关掉隧道的定时检查和升级状态的定时轮询，只留回到前台、网络变化和重试触发的检查。只给测试用。
    #[doc(hidden)]
    pub fn stop_periodic_checks(&self) {
        let scope = self.scope();
        scope.home.stop_periodic_checks();
        scope.plugins.stop_upgrade_polling();
        self.replay().no_periodic_checks = true;
    }

    /// 把当前的隧道 endpoint 关掉，模拟本机 socket 死掉：这条 socket 上的连接和中继一起失效，客户端的
    /// 退避重连拨的全是死 socket。endpoint 重建的触发条件靠它验证。只给测试用。
    #[cfg(feature = "test-hooks")]
    #[doc(hidden)]
    pub fn test_close_home_endpoint(&self) {
        self.scope().home.test_close_endpoint();
    }

    /// 指定 iroh 的网络配置。[`Network::DirectOnly`] 不连中继、不做地址发现，本机和局域网测试用。
    pub fn with_network(data_dir: &str, network: Network) -> Result<Arc<Self>, CoreError> {
        crate::l10n::init();
        let root = PathBuf::from(data_dir);
        let servers = Arc::new(ServerList::open(&root)?);
        let slot = servers.current();
        let scope = Scope::open(
            &slot,
            &root,
            &servers.slot_dir(&slot),
            network,
            servers.subscribe(),
            None,
        )?;
        let switching = Arc::new(Mutex::new(()));
        let visits = Visits::new(servers.clone(), root.clone(), network, switching.clone());
        let servers_for_login = servers.clone();
        let core = Arc::new(Self {
            state: watch::Sender::new(SessionStore::default()),
            appearance: AppearanceLink::new(&root),
            #[cfg(feature = "ui-fake")]
            views: RwLock::new(Views::of(&scope)),
            #[cfg(not(feature = "ui-fake"))]
            views: Views::of(&scope),
            scope: RwLock::new(scope.clone()),
            servers,
            root,
            network,
            replay: Mutex::default(),
            switching,
            visits,
            in_flight_registrations: Arc::default(),
            matrix_login: crate::matrix_login::MatrixLogins::new(servers_for_login),
            #[cfg(feature = "ui-fake")]
            ui_fake: AtomicBool::new(false),
        });
        let switch: std::sync::Weak<Self> = Arc::downgrade(&core);
        core.matrix_login.bind(switch);
        core.attach(&scope);
        Ok(core)
    }

    /// `dispatch` 的后半：插件的操作，其余都是聊天的。分开放是因为函数长度有上限。
    fn dispatch_plugins_or_chat(&self, scope: &Scope, action: Action) -> Result<(), CoreError> {
        match action {
            Action::SetBotName { plugin_id, name } => scope.chat.set_bot_name(plugin_id, name),
            Action::SetBotAvatar {
                plugin_id,
                state,
                upload,
            } => scope.chat.set_bot_avatar(plugin_id, state, upload),
            Action::ClearBotAvatar { plugin_id, state } => {
                scope.chat.clear_bot_avatar(plugin_id, state)
            }
            Action::DismissBotProfileProblem { plugin_id } => {
                scope.chat.dismiss_bot_profile_problem(&plugin_id);
                Ok(())
            }
            Action::SetLanguages { languages } => {
                crate::l10n::set_languages(&languages);
                scope.set_languages(&languages);
                self.replay().languages = Some(languages);
                Ok(())
            }
            Action::RefreshPlugins => {
                scope.plugins.refresh_installed();
                Ok(())
            }
            Action::StartPlugin { plugin_id } => scope.plugins.start(plugin_id),
            Action::SetPortMapping { plugin_id, enabled } => {
                scope.plugins.set_port_mapping(plugin_id, enabled)
            }
            Action::SetVisiblePlugin { plugin_id } => {
                scope.plugins.set_visible_plugin(plugin_id);
                Ok(())
            }
            Action::RefreshStore { force } => {
                scope.plugins.refresh_store(force);
                Ok(())
            }
            Action::InstallPlugin {
                preview,
                secrets,
                settings,
            } => scope.plugins.install(preview, secrets, settings),
            Action::UpgradePlugin {
                preview,
                secrets,
                settings,
            } => scope.plugins.upgrade(preview, secrets, settings),
            Action::InstallNow { source } => scope.plugins.install_now(source),
            Action::UpgradeNow { plugin_id } => scope.plugins.upgrade_now(plugin_id),
            Action::UpgradeAll => scope.plugins.upgrade_all(),
            Action::DismissStoreConfirmation => {
                scope.plugins.dismiss_confirmation();
                Ok(())
            }
            Action::UninstallPlugin { id, delete_data } => scope.plugins.uninstall(id, delete_data),
            Action::DismissPluginTask { id } => scope.plugins.dismiss_task(&id),
            Action::StartTakeover { id } => {
                scope.plugins.start_takeover(id);
                Ok(())
            }
            Action::EndTakeover { id } => {
                scope.plugins.end_takeover(&id);
                Ok(())
            }
            Action::RefreshRegistryMirror => {
                scope.plugins.refresh_registry_mirror();
                Ok(())
            }
            Action::SaveRegistryMirror { mirror } => scope.plugins.save_registry_mirror(mirror),
            Action::TestRegistryMirrors => scope.plugins.test_registry_mirrors(),
            Action::RefreshEgressProxy => {
                scope.plugins.refresh_egress();
                Ok(())
            }
            Action::SaveEgressProxy {
                address,
                direct_cidrs,
                use_plugin,
            } => scope
                .plugins
                .save_egress(&address, &direct_cidrs, use_plugin),
            Action::ConfirmEgressProxy => scope.plugins.confirm_egress(),
            Action::DismissEgressConfirmation => {
                scope.plugins.dismiss_egress_confirmation();
                Ok(())
            }
            Action::AskAgent { text } => {
                let bot = scope.onboarding.view().primary_bot_user_id.ok_or_else(|| {
                    CoreError::NotReady {
                        reason: l10n!(CATALOG, language(), "chat-no-default-agent"),
                    }
                })?;
                scope.chat.ask(&bot, &text)
            }
            chat => scope.chat.dispatch(chat),
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
        reason: l10n!(CATALOG, language(), "home-lan-unsupported"),
    }
}

impl Drop for ClientCore {
    fn drop(&mut self) {
        self.scope().close(false);
    }
}
