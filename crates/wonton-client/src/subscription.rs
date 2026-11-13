use tokio::sync::{Mutex, watch};
use wonton_protocol::{
    AgentChainsViewModel, AppLockViewModel, AppearanceViewModel, BotContactsViewModel,
    ChatViewModel, ComposerViewModel, ConfirmationsViewModel, FriendsViewModel, HomeViewModel,
    OnboardingViewModel, PagesViewModel, PeersViewModel, PluginsViewModel, ServersViewModel,
    SessionListViewModel, SettingsViewModel, TransfersViewModel, WidgetSnapshot,
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

/// Agent 链列表的订阅（ADR-0061 第 10 节），用法同 [`SessionListSubscription`]。
#[derive(uniffi::Object)]
pub struct AgentChainsSubscription {
    updates: Updates<AgentChainsViewModel>,
}

impl AgentChainsSubscription {
    pub(crate) fn new(receiver: watch::Receiver<AgentChainsViewModel>) -> Self {
        Self {
            updates: Updates::new(receiver),
        }
    }
}

#[uniffi::export]
impl AgentChainsSubscription {
    /// 等待下一份链列表，语义同 [`SessionListSubscription::next`]。
    pub async fn next(&self) -> Option<AgentChainsViewModel> {
        self.updates.next(AgentChainsViewModel::clone).await
    }

    /// 取消订阅，语义同 [`SessionListSubscription::cancel`]。
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

/// 未读总数来自聊天房间列表（邀请按 1 计），第一次立即返回；总数归零也会返回 0。
#[derive(uniffi::Object)]
pub struct UnreadSubscription {
    updates: Updates<ChatViewModel>,
}

impl UnreadSubscription {
    pub(crate) fn new(receiver: watch::Receiver<ChatViewModel>) -> Self {
        Self {
            updates: Updates::new(receiver),
        }
    }
}

#[uniffi::export]
impl UnreadSubscription {
    /// 等下一份房间列表的绝对总数；取消或核心释放后返回 None。
    pub async fn next(&self) -> Option<u32> {
        self.updates.next(crate::chat::unread_total).await
    }

    /// 取消订阅，同时结束正在等待的 next。
    pub fn cancel(&self) {
        self.updates.cancel();
    }
}

/// “等你确认”列表的订阅（ADR-0016），用法同 [`SessionListSubscription`]。
#[derive(uniffi::Object)]
pub struct ConfirmationsSubscription {
    updates: Updates<ConfirmationsViewModel>,
}

impl ConfirmationsSubscription {
    pub(crate) fn new(receiver: watch::Receiver<ConfirmationsViewModel>) -> Self {
        Self {
            updates: Updates::new(receiver),
        }
    }
}

#[uniffi::export]
impl ConfirmationsSubscription {
    /// 等待下一份 ViewModel，语义同 [`SessionListSubscription::next`]。
    pub async fn next(&self) -> Option<ConfirmationsViewModel> {
        self.updates.next(ConfirmationsViewModel::clone).await
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
pub struct PagesSubscription {
    updates: Updates<PagesViewModel>,
}

impl PagesSubscription {
    pub(crate) fn new(receiver: watch::Receiver<PagesViewModel>) -> Self {
        Self {
            updates: Updates::new(receiver),
        }
    }
}

#[uniffi::export]
impl PagesSubscription {
    /// 等待下一份 ViewModel，语义同 [`SessionListSubscription::next`]。
    pub async fn next(&self) -> Option<PagesViewModel> {
        self.updates.next(PagesViewModel::clone).await
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

/// App 锁的订阅，用法同 [`SessionListSubscription`]。
#[derive(uniffi::Object)]
pub struct AppLockSubscription {
    updates: Updates<AppLockViewModel>,
}

impl AppLockSubscription {
    pub(crate) fn new(receiver: watch::Receiver<AppLockViewModel>) -> Self {
        Self {
            updates: Updates::new(receiver),
        }
    }
}

#[uniffi::export]
impl AppLockSubscription {
    /// 等待下一份 ViewModel，语义同 [`SessionListSubscription::next`]。
    pub async fn next(&self) -> Option<AppLockViewModel> {
        self.updates.next(AppLockViewModel::clone).await
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

/// Bot 联系人页（ADR-0045、ADR-0050）的订阅，用法同 [`SessionListSubscription`]。
#[derive(uniffi::Object)]
pub struct BotContactsSubscription {
    updates: Updates<BotContactsViewModel>,
}

impl BotContactsSubscription {
    pub(crate) fn new(receiver: watch::Receiver<BotContactsViewModel>) -> Self {
        Self {
            updates: Updates::new(receiver),
        }
    }
}

#[uniffi::export]
impl BotContactsSubscription {
    /// 等待下一份 ViewModel，语义同 [`SessionListSubscription::next`]。
    pub async fn next(&self) -> Option<BotContactsViewModel> {
        self.updates.next(BotContactsViewModel::clone).await
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

/// 一个房间输入框的订阅（ADR-0094），用法同 [`SessionListSubscription`]。
#[derive(uniffi::Object)]
pub struct ComposerSubscription {
    updates: Updates<ComposerViewModel>,
}

impl ComposerSubscription {
    pub(crate) fn new(receiver: watch::Receiver<ComposerViewModel>) -> Self {
        Self {
            updates: Updates::new(receiver),
        }
    }
}

#[uniffi::export]
impl ComposerSubscription {
    /// 等待下一份 ViewModel，语义同 [`SessionListSubscription::next`]。
    pub async fn next(&self) -> Option<ComposerViewModel> {
        self.updates.next(ComposerViewModel::clone).await
    }

    /// 取消订阅，语义同 [`SessionListSubscription::cancel`]。
    pub fn cancel(&self) {
        self.updates.cancel();
    }
}

/// 系统入口快照的订阅（ADR-0100）：每写下一份快照来一次，壳据此让系统刷新小组件、更新快捷方式和搜索索引。
/// 用法同 [`SessionListSubscription`]，只是这个核心启动后还没写过时，第一次 `next()` 等到写下第一份。
#[derive(uniffi::Object)]
pub struct WidgetSnapshotSubscription {
    updates: Updates<Option<WidgetSnapshot>>,
}

impl WidgetSnapshotSubscription {
    pub(crate) fn new(receiver: watch::Receiver<Option<WidgetSnapshot>>) -> Self {
        Self {
            updates: Updates::new(receiver),
        }
    }
}

#[uniffi::export]
impl WidgetSnapshotSubscription {
    /// 等待下一份写下的快照。订阅取消或核心释放后返回 `None`。
    pub async fn next(&self) -> Option<WidgetSnapshot> {
        loop {
            if let Some(snapshot) = self.updates.next(Option::clone).await? {
                return Some(snapshot);
            }
        }
    }

    /// 取消订阅，语义同 [`SessionListSubscription::cancel`]。
    pub fn cancel(&self) {
        self.updates.cancel();
    }
}

#[cfg(test)]
mod unread_tests {
    use super::*;
    use wonton_protocol::{ChatLayout, RoomMembership, RoomRow};

    fn room(unread_count: u64, membership: RoomMembership) -> RoomRow {
        RoomRow {
            room_id: "!room:example.org".into(),
            name: String::new(),
            is_direct: false,
            direct_user_id: None,
            plugin: None,
            is_encrypted: true,
            is_read_only: false,
            membership,
            preview: None,
            side_chats: false,
            unread_count,
            layout: ChatLayout::default(),
        }
    }

    #[tokio::test]
    async fn unread_subscription_uses_rooms_saturates_and_delivers_zero() {
        let dir = tempfile::tempdir().unwrap();
        let core = crate::ClientCore::new(dir.path().to_string_lossy().into_owned()).unwrap();
        let sender = watch::Sender::new(core.chat());
        let subscription = UnreadSubscription::new(sender.subscribe());
        assert_eq!(subscription.next().await, Some(0));
        sender.send_modify(|view| {
            view.rooms = vec![
                room(4, RoomMembership::Joined),
                room(u64::MAX, RoomMembership::Invited),
            ]
        });
        assert_eq!(subscription.next().await, Some(5));
        sender.send_modify(|view| {
            view.rooms = vec![
                room(u64::MAX, RoomMembership::Joined),
                room(u64::MAX, RoomMembership::Joined),
            ]
        });
        assert_eq!(subscription.next().await, Some(u32::MAX));
        sender.send_modify(|view| view.rooms.clear());
        assert_eq!(subscription.next().await, Some(0));
        subscription.cancel();
        assert_eq!(subscription.next().await, None);
    }
}
