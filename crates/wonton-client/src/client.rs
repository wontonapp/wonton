use std::sync::Arc;

use tokio::runtime::Runtime;
use tokio::sync::watch;
use wonton_protocol::{Action, CoreError, HomeViewModel, SessionListViewModel};

use crate::home::{HomeLink, Network, Storage};
use crate::session_store::SessionStore;
use crate::subscription::{HomeSubscription, SessionListSubscription};

/// 客户端核心的入口。原生壳在启动时创建一个实例并一直持有。
///
/// 状态只存在这里：watch 通道既保存当前状态，也负责通知订阅者。网络操作在核心自己的 tokio 运行时
/// 里执行，进度和结果体现在 ViewModel 里。
#[derive(uniffi::Object)]
pub struct ClientCore {
    state: watch::Sender<SessionStore>,
    home: Arc<HomeLink>,
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
            Action::AppForegrounded => {
                self.home.foregrounded();
                Ok(())
            }
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
        Ok(Arc::new(Self {
            state: watch::Sender::new(SessionStore::default()),
            home,
            runtime: Some(runtime),
        }))
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
