use tokio::sync::{Mutex, watch};
use wonton_protocol::{
    AppearanceViewModel, ChatViewModel, FriendsViewModel, HomeViewModel, OnboardingViewModel,
    PeersViewModel, PluginsViewModel, ServersViewModel, SessionListViewModel, SettingsViewModel,
    TabsViewModel, TransfersViewModel,
};

use crate::session_store::SessionStore;

/// 订阅的共用部分：等 watch 通道的下一个值，可以取消。
struct Updates<T> {
    receiver: Mutex<watch::Receiver<T>>,
    cancelled: watch::Sender<bool>,
}

impl<T> Updates<T> {
    fn new(mut receiver: watch::Receiver<T>) -> Self {
        // 让第一次 next() 立即拿到当前值。
        receiver.mark_changed();
        Self {
            receiver: Mutex::new(receiver),
            cancelled: watch::Sender::new(false),
        }
    }

    async fn next<R>(&self, view: impl FnOnce(&T) -> R) -> Option<R> {
        let mut cancelled = self.cancelled.subscribe();
        let mut receiver = self.receiver.lock().await;
        let changed = tokio::select! {
            biased;
            _ = cancelled.wait_for(|cancelled| *cancelled) => return None,
            changed = receiver.changed() => changed,
        };
        changed.ok()?;
        Some(view(&receiver.borrow_and_update()))
    }

    fn cancel(&self) {
        self.cancelled.send_replace(true);
    }
}

/// 会话列表的订阅。壳循环调用 [`next`](Self::next)，包装成 Swift 的 `AsyncSequence`
/// 或 Kotlin 的 `Flow`；不再需要时调用 [`cancel`](Self::cancel)。
///
/// 订阅不持有任何原生对象，核心也不会回调原生代码，所以不存在跨语言的循环引用和重入。
#[derive(uniffi::Object)]
pub struct SessionListSubscription {
    updates: Updates<SessionStore>,
}

impl SessionListSubscription {
    pub(crate) fn new(receiver: watch::Receiver<SessionStore>) -> Self {
        Self {
            updates: Updates::new(receiver),
        }
    }
}

#[uniffi::export]
impl SessionListSubscription {
    /// 等待下一份 ViewModel。第一次调用立即返回当前值；之后在状态变化时返回最新值，
    /// 来不及取走的中间状态会被合并掉。订阅取消或核心释放后返回 `None`。
    pub async fn next(&self) -> Option<SessionListViewModel> {
        self.updates.next(SessionStore::view_model).await
    }

    /// 取消订阅：正在等待的 `next()` 立即返回 `None`，之后的调用也都返回 `None`。
    ///
    /// Kotlin 协程取消时 UniFFI 会释放等待中的 Rust future；Swift 的 Task 取消不会传到
    /// Rust（UniFFI 0.32），所以 Swift 侧要在取消时调用它。
    pub fn cancel(&self) {
        self.updates.cancel();
    }
}

/// 家里服务器连接的订阅，用法同 [`SessionListSubscription`]。
#[derive(uniffi::Object)]
pub struct HomeSubscription {
    updates: Updates<HomeViewModel>,
}

impl HomeSubscription {
    pub(crate) fn new(receiver: watch::Receiver<HomeViewModel>) -> Self {
        Self {
            updates: Updates::new(receiver),
        }
    }
}

#[uniffi::export]
impl HomeSubscription {
    /// 等待下一份 ViewModel，语义同 [`SessionListSubscription::next`]。
    pub async fn next(&self) -> Option<HomeViewModel> {
        self.updates.next(HomeViewModel::clone).await
    }

    /// 取消订阅，语义同 [`SessionListSubscription::cancel`]。
    pub fn cancel(&self) {
        self.updates.cancel();
    }
}

/// 聊天状态的订阅，用法同 [`SessionListSubscription`]。
#[derive(uniffi::Object)]
pub struct ChatSubscription {
    updates: Updates<ChatViewModel>,
}

impl ChatSubscription {
    pub(crate) fn new(receiver: watch::Receiver<ChatViewModel>) -> Self {
        Self {
            updates: Updates::new(receiver),
        }
    }
}

#[uniffi::export]
impl ChatSubscription {
    /// 等待下一份 ViewModel，语义同 [`SessionListSubscription::next`]。
    pub async fn next(&self) -> Option<ChatViewModel> {
        self.updates.next(ChatViewModel::clone).await
    }

    /// 取消订阅，语义同 [`SessionListSubscription::cancel`]。
    pub fn cancel(&self) {
        self.updates.cancel();
    }
}

/// 插件页面的订阅，用法同 [`SessionListSubscription`]。
#[derive(uniffi::Object)]
pub struct PluginsSubscription {
    updates: Updates<PluginsViewModel>,
}

impl PluginsSubscription {
    pub(crate) fn new(receiver: watch::Receiver<PluginsViewModel>) -> Self {
        Self {
            updates: Updates::new(receiver),
        }
    }
}

#[uniffi::export]
impl PluginsSubscription {
    /// 等待下一份 ViewModel，语义同 [`SessionListSubscription::next`]。
    pub async fn next(&self) -> Option<PluginsViewModel> {
        self.updates.next(PluginsViewModel::clone).await
    }

    /// 取消订阅，语义同 [`SessionListSubscription::cancel`]。
    pub fn cancel(&self) {
        self.updates.cancel();
    }
}

/// 底栏和插件页面的订阅，用法同 [`SessionListSubscription`]。
#[derive(uniffi::Object)]
pub struct TabsSubscription {
    updates: Updates<TabsViewModel>,
}

impl TabsSubscription {
    pub(crate) fn new(receiver: watch::Receiver<TabsViewModel>) -> Self {
        Self {
            updates: Updates::new(receiver),
        }
    }
}

#[uniffi::export]
impl TabsSubscription {
    /// 等待下一份 ViewModel，语义同 [`SessionListSubscription::next`]。
    pub async fn next(&self) -> Option<TabsViewModel> {
        self.updates.next(TabsViewModel::clone).await
    }

    /// 取消订阅，语义同 [`SessionListSubscription::cancel`]。
    pub fn cancel(&self) {
        self.updates.cancel();
    }
}

/// 新用户引导的订阅，用法同 [`SessionListSubscription`]。
#[derive(uniffi::Object)]
pub struct OnboardingSubscription {
    updates: Updates<OnboardingViewModel>,
}

impl OnboardingSubscription {
    pub(crate) fn new(receiver: watch::Receiver<OnboardingViewModel>) -> Self {
        Self {
            updates: Updates::new(receiver),
        }
    }
}

#[uniffi::export]
impl OnboardingSubscription {
    /// 等待下一份 ViewModel，语义同 [`SessionListSubscription::next`]。
    pub async fn next(&self) -> Option<OnboardingViewModel> {
        self.updates.next(OnboardingViewModel::clone).await
    }

    /// 取消订阅，语义同 [`SessionListSubscription::cancel`]。
    pub fn cancel(&self) {
        self.updates.cancel();
    }
}

/// 外观的订阅，用法同 [`SessionListSubscription`]。
#[derive(uniffi::Object)]
pub struct AppearanceSubscription {
    updates: Updates<AppearanceViewModel>,
}

impl AppearanceSubscription {
    pub(crate) fn new(receiver: watch::Receiver<AppearanceViewModel>) -> Self {
        Self {
            updates: Updates::new(receiver),
        }
    }
}

#[uniffi::export]
impl AppearanceSubscription {
    /// 等待下一份 ViewModel，语义同 [`SessionListSubscription::next`]。
    pub async fn next(&self) -> Option<AppearanceViewModel> {
        self.updates.next(AppearanceViewModel::clone).await
    }

    /// 取消订阅，语义同 [`SessionListSubscription::cancel`]。
    pub fn cancel(&self) {
        self.updates.cancel();
    }
}

/// 传输列表的订阅，用法同 [`SessionListSubscription`]。
#[derive(uniffi::Object)]
pub struct TransfersSubscription {
    updates: Updates<TransfersViewModel>,
}

impl TransfersSubscription {
    pub(crate) fn new(receiver: watch::Receiver<TransfersViewModel>) -> Self {
        Self {
            updates: Updates::new(receiver),
        }
    }
}

#[uniffi::export]
impl TransfersSubscription {
    /// 等待下一份 ViewModel，语义同 [`SessionListSubscription::next`]。
    pub async fn next(&self) -> Option<TransfersViewModel> {
        self.updates.next(TransfersViewModel::clone).await
    }

    /// 取消订阅，语义同 [`SessionListSubscription::cancel`]。
    pub fn cancel(&self) {
        self.updates.cancel();
    }
}

/// 设置首页的订阅，用法同 [`SessionListSubscription`]。
#[derive(uniffi::Object)]
pub struct SettingsSubscription {
    updates: Updates<SettingsViewModel>,
}

impl SettingsSubscription {
    pub(crate) fn new(receiver: watch::Receiver<SettingsViewModel>) -> Self {
        Self {
            updates: Updates::new(receiver),
        }
    }
}

#[uniffi::export]
impl SettingsSubscription {
    /// 等待下一份 ViewModel，语义同 [`SessionListSubscription::next`]。
    pub async fn next(&self) -> Option<SettingsViewModel> {
        self.updates.next(SettingsViewModel::clone).await
    }

    /// 取消订阅，语义同 [`SessionListSubscription::cancel`]。
    pub fn cancel(&self) {
        self.updates.cancel();
    }
}

/// 配过的服务器列表的订阅，用法同 [`SessionListSubscription`]。
#[derive(uniffi::Object)]
pub struct ServersSubscription {
    updates: Updates<ServersViewModel>,
}

impl ServersSubscription {
    pub(crate) fn new(receiver: watch::Receiver<ServersViewModel>) -> Self {
        Self {
            updates: Updates::new(receiver),
        }
    }
}

#[uniffi::export]
impl ServersSubscription {
    /// 等待下一份 ViewModel，语义同 [`SessionListSubscription::next`]。
    pub async fn next(&self) -> Option<ServersViewModel> {
        self.updates.next(ServersViewModel::clone).await
    }

    /// 取消订阅，语义同 [`SessionListSubscription::cancel`]。
    pub fn cancel(&self) {
        self.updates.cancel();
    }
}

/// “其他服务器”页面（ADR-0050）的订阅，用法同 [`SessionListSubscription`]。
#[derive(uniffi::Object)]
pub struct PeersSubscription {
    updates: Updates<PeersViewModel>,
}

impl PeersSubscription {
    pub(crate) fn new(receiver: watch::Receiver<PeersViewModel>) -> Self {
        Self {
            updates: Updates::new(receiver),
        }
    }
}

#[uniffi::export]
impl PeersSubscription {
    /// 等待下一份 ViewModel，语义同 [`SessionListSubscription::next`]。
    pub async fn next(&self) -> Option<PeersViewModel> {
        self.updates.next(PeersViewModel::clone).await
    }

    /// 取消订阅，语义同 [`SessionListSubscription::cancel`]。
    pub fn cancel(&self) {
        self.updates.cancel();
    }
}

/// 好友页（ADR-0051）的订阅，用法同 [`SessionListSubscription`]。
#[derive(uniffi::Object)]
pub struct FriendsSubscription {
    updates: Updates<FriendsViewModel>,
}

impl FriendsSubscription {
    pub(crate) fn new(receiver: watch::Receiver<FriendsViewModel>) -> Self {
        Self {
            updates: Updates::new(receiver),
        }
    }
}

#[uniffi::export]
impl FriendsSubscription {
    /// 等待下一份 ViewModel，语义同 [`SessionListSubscription::next`]。
    pub async fn next(&self) -> Option<FriendsViewModel> {
        self.updates.next(FriendsViewModel::clone).await
    }

    /// 取消订阅，语义同 [`SessionListSubscription::cancel`]。
    pub fn cancel(&self) {
        self.updates.cancel();
    }
}
