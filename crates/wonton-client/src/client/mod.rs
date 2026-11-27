use std::path::PathBuf;
#[cfg(feature = "ui-fake")]
use std::sync::RwLockReadGuard;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use tokio::sync::watch;
use wonton_l10n::l10n;
use wonton_protocol::{
    AddedSource, AppLockTimeout, AppLockViewModel, AppearanceViewModel, AvatarUpload, ChatContact,
    ChatViewModel, ComposerDraft, CoreError, FamilyViewModel, FileCard, FileCardActions,
    FileFailure, FileId, FileInfo, FileOperation, FileProblem, FriendsViewModel, HomeViewModel,
    InstallPreview, LanChoice, LanSettings, LicensesViewModel, Me, MediaHandle, MyAvatarProblem,
    OnboardingViewModel, OwnerProfileUpdate, PagesViewModel, PeersViewModel, PluginDetail,
    PluginImage, PluginsViewModel, PrivacyViewModel, RoomTransfers, SecretString, ServersViewModel,
    SessionListViewModel, SettingsViewModel, StoreItem, TimelineTarget, TransfersViewModel,
};

use crate::app_lock::AppLockLink;
use crate::appearance::AppearanceLink;
use crate::chat::TimelineSubscription;
use crate::files::FileOpener;
use crate::home::{Network, path_settings};
use crate::l10n::{CATALOG, language};
use crate::scope::{Scope, Views, Visits};
use crate::servers::ServerList;
use crate::session_store::SessionStore;
use crate::subscription::{
    AppLockSubscription, AppearanceSubscription, ChatSubscription, FamilySubscription,
    FriendsSubscription, HomeSubscription, OnboardingSubscription, PagesSubscription,
    PeersSubscription, PluginsSubscription, ServersSubscription, SessionListSubscription,
    SettingsSubscription, TransfersSubscription, UnreadSubscription,
};
use switch::Replay;
#[cfg(feature = "ui-fake")]
use wonton_protocol::{AvatarImage, AvatarKind};

pub use share_in::ShareInSubscription;

mod agent_chains;
mod bot_contacts;
mod browser;
mod composer;
mod confirmations;
mod dispatch;
mod lan;
mod limits;
mod media;
mod my_avatar;
mod page_color;
mod pages;
mod plugin_detail;
mod plugins;
mod profile;
mod push_route;
mod settings;
mod share_in;
mod sharing;
mod switch;
#[cfg(feature = "test-hooks")]
mod test_hooks;
mod test_support;
mod timeline;
mod tool_policy;
mod transfers;
#[cfg(feature = "ui-fake")]
mod ui_fake;
#[cfg(feature = "ui-fake")]
mod ui_fake_agent_stop;
#[cfg(feature = "ui-fake")]
mod ui_fake_devices;
#[cfg(feature = "ui-fake")]
mod ui_fake_failed_install;
#[cfg(feature = "ui-fake")]
mod ui_fake_family;
#[cfg(feature = "ui-fake")]
mod ui_fake_plugin_detail;
#[cfg(feature = "ui-fake")]
mod ui_fake_room;
#[cfg(feature = "ui-fake")]
mod ui_fake_settings;
#[cfg(feature = "ui-fake")]
mod ui_fake_store;
#[cfg(feature = "ui-fake")]
mod ui_fake_timeline;
mod views;
mod widget;

/// 客户端核心的入口。原生壳在启动时创建一个实例并一直持有。
///
/// 状态只存在这里：watch 通道既保存当前状态，也负责通知订阅者。连着服务器的部件都在当前的作用域里
/// （[`Scope`]），网络操作在作用域自己的 tokio 运行时里执行，进度和结果体现在 ViewModel 里。切换服务器时
/// 换掉作用域，壳的订阅接在 `views` 上，不用重建。
#[derive(uniffi::Object)]
pub struct ClientCore {
    state: watch::Sender<SessionStore>,
    /// App 锁（ADR-0102）：整机一个，不随服务器切换重建。
    app_lock: AppLockLink,
    appearance: AppearanceLink,
    servers: Arc<ServerList>,
    root: PathBuf,
    /// 代理 TUN 的假地址是否排除出选路，读页和 bind 都读这个缓存，不读盘；文件仍是真相。
    fake_ip_exclusion: AtomicBool,
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
    #[cfg(feature = "test-hooks")]
    language_refresh_hook: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    /// 为不是当前这台的服务器临时开的作用域。
    visits: Arc<Visits>,
    /// 正在给哪台后台登记哪个推送地址，`push_route` 拿它和落盘记录一起去重。
    in_flight_registrations: push_route::InFlightRegistrations,
    matrix_login: Arc<crate::matrix_login::MatrixLogins>,
    /// 系统分享进来、还没发出去的东西（ADR-0097）：不跟服务器走，换服务器、没配对时都在。
    share_in: Arc<crate::share_in::ShareInLink>,
    /// 系统入口的快照文件（ADR-0100），换服务器不换。
    widget: crate::widget::WidgetFile,
    /// 测试用的假数据入口（`new_ui_fake`）开着：操作不落到作用域，ViewModel 保持塞进去的假数据。
    #[cfg(feature = "ui-fake")]
    ui_fake: AtomicBool,
    /// 假数据入口里打开着的房间时间线，壳的操作按房间找到它再推一批。
    #[cfg(feature = "ui-fake")]
    ui_fake_timelines: ui_fake_timeline::FakeTimelines,
}

#[uniffi::export]
impl ClientCore {
    /// `data_dir` 是 App 的私有数据目录（iOS 的 Application Support、Android 的 `filesDir`），
    /// 存设备密钥和上次的连接信息。上次连过家里的服务器时，创建后自动重连。
    #[uniffi::constructor]
    pub fn new(data_dir: String) -> Result<Arc<Self>, CoreError> {
        Self::with_network(&data_dir, Network::Internet)
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

    /// Pause a language action after dispatch captured its scope, before synchronization.
    #[cfg(feature = "test-hooks")]
    #[doc(hidden)]
    pub fn test_set_language_refresh_hook(&self, hook: Option<Arc<dyn Fn() + Send + Sync>>) {
        *switch::lock(&self.language_refresh_hook) = hook;
    }

    /// 页面配置有落了盘、还没写进账号数据的改动时为真。只给测试读。
    #[cfg(feature = "test-hooks")]
    #[doc(hidden)]
    pub fn test_pages_layout_dirty(&self) -> bool {
        self.scope().pages.test_dirty()
    }

    #[cfg(feature = "test-hooks")]
    fn test_before_language_refresh(&self) {
        let hook = switch::lock(&self.language_refresh_hook).clone();
        if let Some(hook) = hook {
            hook();
        }
    }

    /// 指定 iroh 的网络配置。[`Network::DirectOnly`] 不连中继、不做地址发现，本机和局域网测试用。
    pub fn with_network(data_dir: &str, network: Network) -> Result<Arc<Self>, CoreError> {
        crate::l10n::init();
        let root = PathBuf::from(data_dir);
        let servers = Arc::new(ServerList::open(&root)?);
        let app_lock = AppLockLink::new(&root);
        let slot = servers.current();
        let scope = Scope::open(
            &slot,
            &root,
            &servers.slot_dir(&slot),
            network,
            servers.subscribe(),
            &app_lock,
            None,
        )?;
        let switching = Arc::new(Mutex::new(()));
        let visits = Visits::new(
            servers.clone(),
            root.clone(),
            network,
            switching.clone(),
            app_lock.clone(),
        );
        let servers_for_login = servers.clone();
        let share_in = Arc::new(crate::share_in::ShareInLink::new(&root));
        let fake_ip_exclusion = path_settings::exclude_fake_ip(&root);
        let core = Arc::new(Self {
            state: watch::Sender::new(SessionStore::default()),
            app_lock,
            appearance: AppearanceLink::new(&root),
            widget: crate::widget::WidgetFile::new(&root),
            #[cfg(feature = "ui-fake")]
            views: RwLock::new(Views::of(&scope)),
            #[cfg(not(feature = "ui-fake"))]
            views: Views::of(&scope),
            scope: RwLock::new(scope.clone()),
            servers,
            root,
            network,
            fake_ip_exclusion: AtomicBool::new(fake_ip_exclusion),
            replay: Mutex::default(),
            switching,
            #[cfg(feature = "test-hooks")]
            language_refresh_hook: Mutex::default(),
            visits,
            in_flight_registrations: Arc::default(),
            matrix_login: crate::matrix_login::MatrixLogins::new(servers_for_login),
            share_in,
            #[cfg(feature = "ui-fake")]
            ui_fake: AtomicBool::new(false),
            #[cfg(feature = "ui-fake")]
            ui_fake_timelines: ui_fake_timeline::FakeTimelines::new(
                scope.handle().expect("the scope was just built"),
            ),
        });
        let switch: std::sync::Weak<Self> = Arc::downgrade(&core);
        core.matrix_login.bind(switch);
        core.attach(&scope);
        Ok(core)
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
        self.scope().close(false);
    }
}
