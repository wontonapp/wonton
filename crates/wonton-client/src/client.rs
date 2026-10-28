use std::sync::Arc;

use tokio::sync::watch;
use wonton_protocol::{Action, CoreError, SessionListViewModel};

use crate::session_store::SessionStore;
use crate::subscription::SessionListSubscription;

/// 客户端核心的入口。原生壳在启动时创建一个实例并一直持有。
///
/// 状态只存在这里：watch 通道既保存当前状态，也负责通知订阅者。
#[derive(uniffi::Object)]
pub struct ClientCore {
    state: watch::Sender<SessionStore>,
}

#[uniffi::export]
impl ClientCore {
    #[uniffi::constructor]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            state: watch::Sender::new(SessionStore::default()),
        })
    }

    /// 处理一个操作。成功时状态立即更新并推给所有订阅者；失败时状态不变，返回原因。
    ///
    /// 这个调用不阻塞，可以在主线程上直接调用。以后的网络操作在核心内部异步执行，
    /// 进度和结果体现在 ViewModel 里。
    pub fn dispatch(&self, action: Action) -> Result<(), CoreError> {
        let mut outcome = Ok(());
        self.state
            .send_if_modified(|store| match store.apply(action) {
                Ok(()) => true,
                Err(error) => {
                    outcome = Err(error);
                    false
                }
            });
        outcome
    }

    /// 当前的会话列表，用于首帧渲染。
    pub fn session_list(&self) -> SessionListViewModel {
        self.state.borrow().view_model()
    }

    /// 订阅会话列表。第一次 `next()` 立即返回当前值，之后每次状态变化返回最新值。
    pub fn subscribe_session_list(&self) -> Arc<SessionListSubscription> {
        Arc::new(SessionListSubscription::new(self.state.subscribe()))
    }
}
