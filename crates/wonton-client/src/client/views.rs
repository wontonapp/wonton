//! 界面状态的读取和订阅：每个 ViewModel 的当前值和 watch 通道入口，拆出 [client mod] 保持文件在门禁内。

use super::*;

#[uniffi::export]
impl ClientCore {
    /// 当前的底栏和插件页面，用于首帧渲染。
    pub fn pages(&self) -> PagesViewModel {
        self.views().pages.borrow().clone()
    }
    /// 订阅底栏，语义同 [`subscribe_session_list`](Self::subscribe_session_list)。
    pub fn subscribe_pages(&self) -> Arc<PagesSubscription> {
        Arc::new(PagesSubscription::new(self.views().pages.subscribe()))
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
    /// 当前的成员页（ADR-0088），用于首帧渲染。
    pub fn family(&self) -> FamilyViewModel {
        self.views().family.borrow().clone()
    }
    /// 订阅成员页，语义同 [`subscribe_session_list`](Self::subscribe_session_list)。
    pub fn subscribe_family(&self) -> Arc<FamilySubscription> {
        Arc::new(FamilySubscription::new(self.views().family.subscribe()))
    }

    /// 当前的“HTTPS 和域名”设置页（ADR-0089），用于首帧渲染。
    pub fn clients_domain(&self) -> ClientsDomainViewModel {
        self.views().clients_domain.borrow().clone()
    }

    /// 订阅“HTTPS 和域名”设置页，语义同 [`subscribe_session_list`](Self::subscribe_session_list)。
    pub fn subscribe_clients_domain(&self) -> Arc<ClientsDomainSubscription> {
        Arc::new(ClientsDomainSubscription::new(
            self.views().clients_domain.subscribe(),
        ))
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
    /// 当前服务器房间的未读总数；邀请按 1 计，溢出饱和到 u32::MAX。打开 App 不会清零。
    pub fn unread_total(&self) -> u32 {
        crate::chat::unread_total(&self.views().chat.borrow())
    }
    /// 订阅同一份房间事实源的绝对未读总数；切换服务器后跟随新的服务器。
    pub fn subscribe_unread(&self) -> Arc<UnreadSubscription> {
        Arc::new(UnreadSubscription::new(self.views().chat.subscribe()))
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
}
