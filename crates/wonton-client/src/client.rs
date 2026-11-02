use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use tokio::runtime::Runtime;
use tokio::sync::watch;
use wonton_protocol::{
    Action, ChatViewModel, CoreError, HomeViewModel, InstallPreview, LanChoice, LanSettings,
    MediaHandle, PluginDetail, PluginFailure, PluginImage, PluginProblem, PluginsViewModel,
    PushNotification, SessionListViewModel, StoreItem,
};

use crate::chat::{ChatLink, TimelineSubscription};
use crate::home::{HomeLink, Network, Storage};
use crate::push::PushLink;
use crate::session_store::SessionStore;
use crate::subscription::{
    ChatSubscription, HomeSubscription, PluginsSubscription, SessionListSubscription,
};

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
    plugins: watch::Sender<PluginsViewModel>,
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
            Action::AppForegrounded => {
                self.home.foregrounded();
                self.chat.foregrounded();
                self.push.foregrounded();
                Ok(())
            }
            Action::SetPushEndpoint { endpoint } => self.push.set_endpoint(endpoint),
            Action::PushUnavailable { message } => {
                self.push.unavailable(message);
                Ok(())
            }
            Action::SetLanguages { .. }
            | Action::RefreshPlugins
            | Action::RefreshStore { .. }
            | Action::InstallPlugin { .. }
            | Action::UpgradePlugin { .. }
            | Action::UninstallPlugin { .. }
            | Action::DismissPluginTask { .. } => Err(plugins_unsupported()),
            chat => self.chat.dispatch(chat),
        }
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
        self.plugins.borrow().clone()
    }

    /// 订阅插件页面，语义同 [`subscribe_session_list`](Self::subscribe_session_list)。
    pub fn subscribe_plugins(&self) -> Arc<PluginsSubscription> {
        Arc::new(PluginsSubscription::new(self.plugins.subscribe()))
    }

    /// 商店里一个插件的详情（来源最新的版本）。`source` 是 [`StoreItem::source`]。
    pub async fn store_plugin(&self, source: String) -> Result<PluginDetail, CoreError> {
        let _ = source;
        Err(plugins_unsupported())
    }

    /// 一个已安装插件的详情（装着的版本）。
    pub async fn installed_plugin(&self, id: String) -> Result<PluginDetail, CoreError> {
        let _ = id;
        Err(plugins_unsupported())
    }

    /// 安装前的预览：家里的服务器解析来源、取插件包，给出要主人确认的权限和说明，确认页渲染它。`source` 是
    /// [`StoreItem::source`]，或者主人输入的 `github:<owner>/<repo>[@<tag>]`。
    pub async fn preview_install(&self, source: String) -> Result<InstallPreview, CoreError> {
        let _ = source;
        Err(plugins_unsupported())
    }

    /// 升级前的预览：升到已安装列表里提示的版本。
    pub async fn preview_upgrade(&self, id: String) -> Result<InstallPreview, CoreError> {
        let _ = id;
        Err(plugins_unsupported())
    }

    /// 取插件的图标或截图：PNG 或 JPEG 的字节，经隧道取回并缓存，同 [`load_media`](Self::load_media)。
    pub async fn load_plugin_image(&self, image: PluginImage) -> Result<Vec<u8>, CoreError> {
        let _ = image;
        Err(plugins_unsupported())
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
        let _ = query;
        Vec::new()
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
        let chat = ChatLink::new(runtime.handle().clone(), home.clone(), Path::new(data_dir))?;
        let push = PushLink::new(runtime.handle().clone(), home.clone(), chat.clone());
        Ok(Arc::new(Self {
            state: watch::Sender::new(SessionStore::default()),
            home,
            chat,
            push,
            plugins: watch::Sender::new(PluginsViewModel {
                problem: Some(PluginProblem {
                    reason: PluginFailure::Protocol,
                    message: plugins_unsupported().to_string(),
                }),
                ..PluginsViewModel::default()
            }),
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

/// 插件商店的接口先于实现合并（ADR-0017）：实现之前，插件相关的调用都如实失败。
fn plugins_unsupported() -> CoreError {
    CoreError::NotReady {
        reason: "这个版本的客户端核心不支持插件商店".into(),
    }
}

/// 局域网设置的接口同样先于实现合并（ADR-0019）。
fn lan_unsupported() -> CoreError {
    CoreError::NotReady {
        reason: "这个版本的客户端核心不支持局域网设置".into(),
    }
}

impl Drop for ClientCore {
    fn drop(&mut self) {
        // 不等后台任务结束：壳可能在主线程上释放核心，测试可能在异步上下文里释放。
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_background();
        }
    }
}
