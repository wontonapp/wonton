//! 客户端厚核心：状态、业务逻辑、网络与 ViewModel，经 UniFFI 导出给 SwiftUI 与 Compose。
//!
//! 接口形态是“操作进、ViewModel 出”，并保持可序列化，以便测试和将来的 Web 端复用。
//! 设计见 `docs/adr/0005-native-clients-thick-core.md`。
//!
//! - 操作进：壳调用 [`ClientCore::dispatch`]，参数是 [`Action`]。
//! - ViewModel 出：壳调用 [`ClientCore::subscribe_session_list`]、[`ClientCore::subscribe_home`]、
//!   [`ClientCore::subscribe_chat`]、[`ClientCore::subscribe_plugins`]、[`ClientCore::subscribe_onboarding`]、[`ClientCore::subscribe_transfers`]、[`ClientCore::subscribe_appearance`]，循环等待订阅的 `next()`，包装成 Swift 的 `AsyncSequence` 或 Kotlin 的 `Flow`。打开的房间的时间线用 [`ClientCore::subscribe_timeline`]，推的是不能合并的
//!   增量批次。
//!
//! 和家里服务器的连接见 `home` 模块：iroh 隧道加 WebView 和 HTTP 客户端用的本地代理（ADR-0004）。聊天见
//! `chat` 模块：经隧道登录主人的 Matrix 账号（ADR-0003、ADR-0009）。推送见 `push` 模块：登记推送订阅，收到推送
//! 时取回消息在本机解密（ADR-0003 的“设计：加密推送”）。文件见 `files` 模块：传输列表、卡片和给播放器的地址（ADR-0020）。会话列表还是 M0 验证 UniFFI 用的示例，
//! 只放在内存里。
//!
//! 引导卡片由 [`OnboardingViewModel::nav`] 控制按钮；账号登录仍直接发 [`Action::ContinueOnboarding`]。
//! 上一步不撤销已保存的资料、默认 Agent、key 或安装，也保留本机选项；已配置模型时，反向导航跳回 Agent，
//! 不把主人送回 Apps。插件安装完成会静默更新推荐行，保存 key 和已有错误不受影响（ADR-0034、ADR-0038）。
//! 后台通知不倒退已发布的引导状态；新操作接替旧操作的忙碌标记，失败后仍能返回或跳过。

// matrix-sdk 的 future 嵌套很深，默认的 128 层不够算出聊天后台任务的类型布局。
#![recursion_limit = "256"]

uniffi::setup_scaffolding!();

#[cfg(target_os = "android")]
mod android;
mod appearance;
mod chat;
mod client;
mod files;
mod home;
mod link;
mod logging;
mod onboarding;
mod plugins;
mod push;
mod servers;
mod session_store;
mod settings;
mod subscription;
mod tabs;

pub use appearance::{accent_hex, default_appearance, parse_accent};
pub use chat::TimelineSubscription;
pub use client::ClientCore;
pub use files::{FileOpenError, FileOpener};
pub use home::Network;
pub use link::{app_link_url, parse_app_link};
pub use logging::init_logging;
pub use subscription::{
    AppearanceSubscription, ChatSubscription, HomeSubscription, OnboardingSubscription,
    PluginsSubscription, ServersSubscription, SessionListSubscription, SettingsSubscription,
    TabsSubscription, TransfersSubscription,
};
pub use tabs::bridge::{
    TabBridgeAction, TabBridgeResult, TabEnvironment, TabPlatform, TabSafeArea,
};
pub use wonton_protocol::LanClient;
pub use wonton_protocol::StoreUnreadable;
pub use wonton_protocol::{
    Action, AddedSource, AddedTools, AgentLogin, Answer, AppLink, AppearanceProblem,
    AppearanceViewModel, BackPagination, BackupReminder, BotSwitch, CandidatePlugin, CardChoice,
    CardLine, ChatAccount, ChatFailure, ChatProblem, ChatSync, ChatViewModel, ConfirmationCard,
    ConfirmationState, ConfirmationView, ConnectionNotice, CoreError, CoreUpgradeCheck,
    CoreUpgradeFailure, CoreUpgradeOffer, CoreUpgradePhase, CoreUpgradeProblem,
    CoreUpgradeProgress, CoreUpgradeRun, CoreUpgradeRunState, CoreUpgradeStage, CoreUpgradeStatus,
    CoreUpgradeTarget, CoreUpgradeViewModel, CoreVersion, DeviceInvite, DeviceNetwork,
    DeviceVerification, EgressConfirmation, EgressCurrent, EgressFailure, EgressPathState,
    EgressProbe, EgressProblem, EgressProxyProvider, EgressProxySettings, EgressProxySource,
    EgressProxyUpdate, EgressRestart, EgressViewModel, EncryptionStatus, FileCard, FileCardAction,
    FileCardActions, FileFailure, FileId, FileInfo, FileOperation, FileProblem, FileSource,
    FontStyle, HomeFailure, HomePath, HomeStatus, HomeViewModel, ImageContent, ImageThumbnail,
    ImageUpload, InstallPreview, InstalledPlugin, InstalledPlugins, LanAddress, LanChoice,
    LanFailure, LanInterface, LanInterfaceKind, LanPlugin, LanPort, LanProblem, LanProtocol,
    LanSettings, LanWebuiOffer, LanWebuiUpdate, MediaHandle, MessageContent, MessageSummary,
    ModelProvider, ModelProviderFailure, ModelProviderInput, ModelProviderProblem, ModelProviders,
    NotificationCategory, OnboardingAgentRow, OnboardingCommit, OnboardingItemProblem,
    OnboardingItemState, OnboardingModelChoice, OnboardingModelStep, OnboardingNeedRow,
    OnboardingProfileStep, OnboardingReview, OnboardingReviewDetail, OnboardingReviewItem,
    OnboardingReviewKind, OnboardingStep, OnboardingTip, OnboardingTips, OnboardingViewModel,
    OpenWindow, OwnerProfile, OwnerProfileUpdate, PairingFailure, PairingStatus,
    PendingConfirmation, PermissionLine, PermissionRisk, PluginAuthor, PluginDetail, PluginDevice,
    PluginFailure, PluginHealth, PluginIcon, PluginImage, PluginKind, PluginOperation,
    PluginProblem, PluginState, PluginTab, PluginTakeover, PluginTask, PluginTaskState,
    PluginTrust, PluginsViewModel, ProxySettings, PushConfirmation, PushContent, PushEndpoint,
    PushNotification, PushStatus, RecoveryStatus, RegistryFailure, RegistryMirrorSettings,
    RegistryMirrorTest, RegistryMirrorTestRequest, RegistryMirrorTests, RegistryMirrorUpdate,
    RegistryMirrorViewModel, RegistryProbe, RegistryProblem, ReplacedAgent, RequirementLine,
    RequirementStatus, Rgb, RoomMembership, RoomPreview, RoomRow, RoomTransfers, SecretString,
    SendState, ServerRow, ServersViewModel, SessionListViewModel, SessionRow, SettingsEntry,
    SettingsSection, SettingsViewModel, SourceKind, StepNav, StoreFailure, StoreItem, StoreListing,
    StoreProblem, StoreSection, TabCandidate, TabDisplay, TabDisplayChoice, TabIcon, TabItem,
    TabLayout, TabLoadFailure, TabPage, TabRef, TabState, TabsProblem, TabsViewModel,
    TakeoverState, TextSize, ThemeColors, ThemeMode, TimelineContent, TimelineDiff, TimelineItem,
    TimelineMessage, TimelineUpdate, ToolAccess, ToolPolicyEntry, ToolPolicyView, Transfer,
    TransferAction, TransferDirection, TransferElsewhere, TransferState, TransfersViewModel,
    UpgradeOffer, UploadSource, VerificationCancel, VerificationEmoji, VerificationFlow,
    VerificationStage, WindowView,
};
