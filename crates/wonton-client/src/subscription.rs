use tokio::sync::{Mutex, watch};
use wonton_protocol::SessionListViewModel;

use crate::session_store::SessionStore;

/// 会话列表的订阅。壳循环调用 [`next`](Self::next)，包装成 Swift 的 `AsyncSequence`
/// 或 Kotlin 的 `Flow`；不再需要时调用 [`cancel`](Self::cancel)。
///
/// 订阅不持有任何原生对象，核心也不会回调原生代码，所以不存在跨语言的循环引用和重入。
#[derive(uniffi::Object)]
pub struct SessionListSubscription {
    receiver: Mutex<watch::Receiver<SessionStore>>,
    cancelled: watch::Sender<bool>,
}

impl SessionListSubscription {
    pub(crate) fn new(mut receiver: watch::Receiver<SessionStore>) -> Self {
        // 让第一次 next() 立即拿到当前值。
        receiver.mark_changed();
        Self {
            receiver: Mutex::new(receiver),
            cancelled: watch::Sender::new(false),
        }
    }
}

#[uniffi::export]
impl SessionListSubscription {
    /// 等待下一份 ViewModel。第一次调用立即返回当前值；之后在状态变化时返回最新值，
    /// 来不及取走的中间状态会被合并掉。订阅取消或核心释放后返回 `None`。
    pub async fn next(&self) -> Option<SessionListViewModel> {
        let mut cancelled = self.cancelled.subscribe();
        let mut receiver = self.receiver.lock().await;
        let changed = tokio::select! {
            biased;
            _ = cancelled.wait_for(|cancelled| *cancelled) => return None,
            changed = receiver.changed() => changed,
        };
        changed.ok()?;
        Some(receiver.borrow_and_update().view_model())
    }

    /// 取消订阅：正在等待的 `next()` 立即返回 `None`，之后的调用也都返回 `None`。
    ///
    /// Kotlin 协程取消时 UniFFI 会释放等待中的 Rust future；Swift 的 Task 取消不会传到
    /// Rust（UniFFI 0.32），所以 Swift 侧要在取消时调用它。
    pub fn cancel(&self) {
        self.cancelled.send_replace(true);
    }
}
