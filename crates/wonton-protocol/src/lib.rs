//! 核心 API、ViewModel 与插件清单（`x-wonton`）的类型定义，是服务端、客户端和插件之间的唯一事实源。
//!
//! 修改这里的公开类型属于跨模块接口变更：先单独提 PR 合并，再由依赖方跟进（见 AGENTS.md）。
//!
//! 所有类型都可以用 serde 序列化，供测试和将来的 Web 端使用；启用 `uniffi` feature 后，
//! 同一份类型经 UniFFI 生成 Swift / Kotlin 类型，原生壳不再手写一份。

#[cfg(feature = "uniffi")]
uniffi::setup_scaffolding!();

mod action;
mod agent_permission;
mod appearance;
mod bot_profile;
mod chat;
mod confirmation;
mod core_upgrade;
mod egress;
mod error;
mod family;
mod file;
mod home;
mod lan;
mod link;
mod onboarding;
mod owner;
mod plugin;
mod plugin_notice;
mod push;
mod registry_mirror;
mod secret;
mod servers;
mod session;
mod settings;
mod tabs;
mod timeline;
mod verification;

pub use action::Action;
pub use agent_permission::{
    AGENT_PERMISSION_PATH, AGENT_PERMISSION_TTL_SECS, AgentPermissionKind, AgentPermissionOption,
    AgentPermissionOutcome, AgentPermissionRequest,
};
pub use appearance::{
    AppearanceProblem, AppearanceViewModel, FontStyle, Rgb, TextSize, ThemeColors, ThemeMode,
};
pub use bot_profile::{
    AVATARS_FIELD, AvatarChange, AvatarImage, AvatarKind, AvatarMedia, AvatarMediaProblem,
    AvatarUpload, BOT_ACTIVITY_PATH, BOT_ACTIVITY_WAIT_SECS, BotActivity, BotActivityFacts,
    BotActivityReport, BotNameProblem, BotProfileFailure, BotProfileProblem, BotProfileUpdate,
    BotProfileUpdated, BotView, MAX_AVATAR_BYTES, MAX_AVATAR_DURATION_MS, MAX_AVATAR_EDGE,
    MAX_BOT_NAME_CHARS, StateAvatar, StateAvatars, bot_profile_path, check_bot_name,
};
pub use chat::{
    BackupReminder, ChatAccount, ChatContact, ChatFailure, ChatProblem, ChatSync, ChatViewModel,
    DeviceVerification, EncryptionStatus, ImageThumbnail, ImageUpload, MessageSummary,
    PendingConfirmation, RecoveryStatus, RoomMembership, RoomPreview, RoomRow,
};
pub use confirmation::{
    Answer, CARD_FIELD, CORE_LOCALPART, CardChoice, CardLine, ConfirmationAnswer, ConfirmationCard,
    ConfirmationList, ConfirmationReply, ConfirmationState, ConfirmationView, ConfirmationWindow,
    WindowEnd, WindowView, answer_path, window_path,
};
pub use core_upgrade::{
    CORE_UPGRADE_CHECK_PATH, CORE_UPGRADE_PATH, CoreUpgradeCheck, CoreUpgradeCheckRequest,
    CoreUpgradeFailure, CoreUpgradeOffer, CoreUpgradePhase, CoreUpgradeProblem,
    CoreUpgradeProgress, CoreUpgradeRequest, CoreUpgradeRun, CoreUpgradeRunState, CoreUpgradeStage,
    CoreUpgradeStatus, CoreUpgradeTarget, CoreUpgradeViewModel, CoreVersion,
};
pub use egress::{
    EgressConfirmation, EgressCurrent, EgressFailure, EgressPathState, EgressProbe, EgressProblem,
    EgressProxyProvider, EgressProxySettings, EgressProxySource, EgressProxyUpdate, EgressRestart,
    EgressViewModel, LanWebuiUpdate,
};
pub use error::CoreError;
pub use family::{
    BotContact, BotContacts, BotKeeper, BotRef, ContactRequest, ContactState, FAMILY_ROOM_EVENT,
    FamilyAccess, GATEWAY_REFUSAL_HEADER, GatewayRefusal, ME_PATH, MEMBER_PAIRING_CODE_TTL_SECS,
    MEMBERS_PATH, Me, Member, MemberInvite, MemberList, MemberPairingCode, MemberRemoval,
    MemberSettings, MemberStatus, NewSharedBot, Role, SHARED_BOTS_PATH, TOOL_CALL_USER_META,
    bot_contact_path, bot_contacts_path, member_pairing_code_path, member_path,
    member_removal_path, plugin_family_path,
};
pub use file::{
    FILE_ERROR_HEADER, FILES_HEADER, FileCard, FileCardAction, FileCardActions, FileError,
    FileErrorBody, FileFailure, FileGrantRequest, FileId, FileInfo, FileLinks, FileOperation,
    FileProblem, FileSource, InvalidFileId, RoomTransfers, Transfer, TransferAction,
    TransferDirection, TransferElsewhere, TransferState, TransfersViewModel, UploadSource,
    extract_file_links,
};
pub use home::{
    ConnectionCheck, ConnectionDetails, ConnectionNotice, DeviceInvite, DeviceNetwork, HomeFailure,
    HomePath, HomeStatus, HomeViewModel, PairingFailure, PairingStatus, ProxySettings,
};
pub use lan::{
    LanAddress, LanChoice, LanFailure, LanInterface, LanInterfaceKind, LanPlugin, LanPort,
    LanProblem, LanProtocol, LanSettings,
};
pub use link::AppLink;
pub use onboarding::{
    AgentLogin, CUSTOM_MODEL_PROVIDER_PATH, CUSTOM_MODEL_PROVIDER_TEST_PATH,
    ConfiguredModelProvider, CustomModelProvider, MODEL_PROVIDERS_PATH, ModelProvider,
    ModelProviderFailure, ModelProviderInput, ModelProviderKey, ModelProviderProblem,
    ModelProviders, ONBOARDING_PATH, Onboarding, OnboardingAgent, OnboardingAgentRow,
    OnboardingCommit, OnboardingItemProblem, OnboardingItemState, OnboardingModelChoice,
    OnboardingModelStep, OnboardingNeed, OnboardingNeedRow, OnboardingProfileStep,
    OnboardingReview, OnboardingReviewDetail, OnboardingReviewItem, OnboardingReviewKind,
    OnboardingStep, OnboardingTip, OnboardingTips, OnboardingUpdate, OnboardingViewModel,
    ReplacedAgent, StepNav, model_provider_path, model_provider_test_path,
};
pub use owner::{
    OWNER_PROFILE_PATH, OwnerProfile, OwnerProfileUpdate, check_display_name, check_email,
    check_email_change, normalized_email,
};
pub use plugin::LanClient;
pub use plugin::StoreUnreadable;
pub use plugin::{
    AddSourceRequest, AddedSource, AddedTools, AskEveryUpdate, BotSwitch, InstallPreview,
    InstallRequest, InstallSource, InstalledPlugin, InstalledPlugins, LanWebuiOffer,
    MAX_PLUGIN_ARCHIVE_BYTES, OpenWindow, PermissionLine, PermissionRisk, PluginAuthor,
    PluginDetail, PluginDevice, PluginFailure, PluginHealth, PluginIcon, PluginImage, PluginKind,
    PluginLimits, PluginOperation, PluginProblem, PluginState, PluginTakeover, PluginTask,
    PluginTaskState, PluginTrust, PluginWriter, PluginsViewModel, RequirementLine,
    RequirementStatus, SourceKind, StorageUsage, StoreFailure, StoreItem, StoreListing,
    StoreProblem, StoreSection, TakeoverState, ToolAccess, ToolPolicyEntry, ToolPolicyView,
    UpgradeOffer, UpgradeRequest, UpgradeResult,
};
pub use plugin_notice::{NOTICE_FIELD, NoticeLevel, PluginNotice};
pub use push::{
    NotificationCategory, PushConfirmation, PushContent, PushEndpoint, PushNotification, PushStatus,
};
pub use registry_mirror::{
    RegistryFailure, RegistryMirrorSettings, RegistryMirrorTest, RegistryMirrorTestRequest,
    RegistryMirrorTests, RegistryMirrorUpdate, RegistryMirrorViewModel, RegistryProbe,
    RegistryProblem,
};
pub use secret::SecretString;
pub use servers::{ServerRow, ServersViewModel};
pub use session::{SessionListViewModel, SessionRow};
pub use settings::{SettingsEntry, SettingsSection, SettingsViewModel};
pub use tabs::{
    CandidatePlugin, PluginTab, TabCandidate, TabDisplay, TabDisplayChoice, TabIcon, TabItem,
    TabLayout, TabLoadFailure, TabPage, TabRef, TabState, TabsProblem, TabsViewModel,
};
pub use timeline::{
    BackPagination, ImageContent, MediaHandle, MessageAction, MessageContent, RichBlock,
    RichBlockKind, RichSpan, SendState, TimelineContent, TimelineDiff, TimelineItem,
    TimelineMessage, TimelineUpdate,
};
pub use verification::{
    VerificationCancel, VerificationEmoji, VerificationFlow, VerificationStage,
};
