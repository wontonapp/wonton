use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use tokio::runtime::Runtime;
use tokio::sync::watch;
use wonton_protocol::{
    Action, ChatViewModel, CoreError, HomeViewModel, MediaHandle, PushNotification,
    SessionListViewModel,
};

use crate::chat::{ChatLink, TimelineSubscription};
use crate::home::{HomeLink, Network, Storage};
use crate::push::PushLink;
use crate::session_store::SessionStore;
use crate::subscription::{ChatSubscription, HomeSubscription, SessionListSubscription};

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
            Action::PairHome { .. } => Err(CoreError::NotReady {
                reason: "这个版本的客户端核心不支持扫码配对".to_owned(),
            }),
            Action::InviteDevice => Err(CoreError::NotReady {
                reason: "这个版本的客户端核心不支持添加设备".to_owned(),
            }),
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

impl Drop for ClientCore {
    fn drop(&mut self) {
        // 不等后台任务结束：壳可能在主线程上释放核心，测试可能在异步上下文里释放。
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_background();
        }
    }
}
