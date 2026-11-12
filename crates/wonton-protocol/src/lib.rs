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
mod agent_run;
mod appearance;
mod bot_commands;
mod bot_contacts;
mod bot_profile;
mod bot_web;
mod bot_web_registration;
mod chat;
mod chat_layout;
mod composer;
mod confirmation;
mod confirmation_origin;
mod core_upgrade;
mod egress;
mod error;
mod family;
mod file;
mod friends;
mod home;
mod lan;
mod link;
mod link_preview;
mod matrix_login;
mod onboarding;
mod owner;
mod panel;
mod peers;
mod plugin;
mod plugin_mail;
mod plugin_notice;
mod port_mapping;
mod push;
mod registry_mirror;
mod reply;
mod roles;
mod secret;
mod server_version;
mod servers;
mod session;
mod settings;
mod share_transport;
mod sharing;
mod sidebar;
mod tabs;
mod timeline;
mod verification;

pub use action::Action;
pub use agent_permission::{
    AGENT_PERMISSION_PATH, AGENT_PERMISSION_TTL_SECS, AgentPermissionKind, AgentPermissionOption,
    AgentPermissionOutcome, AgentPermissionRequest,
};
pub use agent_run::{AgentRunChoice, AgentRunEnd, AgentRunState};
pub use appearance::{
    AppearanceProblem, AppearanceViewModel, FontStyle, Rgb, TextSize, ThemeColors, ThemeMode,
};
pub use bot_commands::{
    BOT_COMMANDS_EVENT, BotCommand, BotCommands, MAX_BOT_COMMANDS, MAX_BOT_COMMANDS_BYTES,
    MAX_COMMAND_DESCRIPTION_CHARS, MAX_COMMAND_HINT_CHARS, MAX_COMMAND_NAME_CHARS,
};
pub use bot_contacts::{
    BOT_CONTACT_CREATE_KEY, BotContact, BotContacts, BotContactsState, BotContactsViewModel,
    CONTACT_ERROR_HEADER, CONTACT_EVENT, CONTACT_ROOM_TYPE, ContactError, ContactErrorBody,
    ContactEvent, ContactKind, ContactOutcome, ContactProblem, ContactRequest, ContactRequestPush,
    ContactState, PendingContactRequest, RemoteContact, bot_contact_approve_path, bot_contact_path,
    bot_contact_reject_path, bot_contacts_path,
};
pub use bot_profile::{
    AVATARS_FIELD, AccessSource, AvatarChange, AvatarImage, AvatarKind, AvatarMedia,
    AvatarMediaProblem, AvatarUpload, BOT_ACTIVITY_PATH, BOT_ACTIVITY_WAIT_SECS, BotActivity,
    BotActivityFacts, BotActivityReport, BotNameProblem, BotPluginAccess, BotPluginAccessUpdate,
    BotProfileFailure, BotProfileProblem, BotProfileUpdate, BotProfileUpdated, BotView,
    MAX_AVATAR_BYTES, MAX_AVATAR_DURATION_MS, MAX_AVATAR_EDGE, MAX_BOT_NAME_CHARS, StateAvatar,
    StateAvatars, bot_plugin_access_path, bot_profile_path, check_bot_name,
};
pub use bot_web::{
    BOT_WEB_LABEL_PREFIX, BotWebFailure, BotWebPage, BotWebView, bot_web_label, bot_web_path,
};
pub use bot_web_registration::{BotWebConflict, bot_web_registration};
pub use chat::{
    BackupReminder, BotInstallRow, BotInstallState, ChatAccount, ChatContact, ChatFailure,
    ChatProblem, ChatSync, ChatViewModel, DeviceVerification, EncryptionStatus, ImageThumbnail,
    ImageUpload, MessageSummary, PendingConfirmation, RecoveryNextStep, RecoveryStatus,
    RoomMembership, RoomPreview, RoomRow,
};
pub use chat_layout::{
    BotChatSettings, CHAT_LAYOUT_EVENT, ChatForm, ChatLayout, ChatLayoutChoice, ChatLayoutSettings,
    ChatLayoutTarget, ChatTextScale, CodeOverflow, PluginChat,
};
pub use composer::{
    AttachmentFailure, AttachmentKind, AttachmentState, ComposerAttachment, ComposerViewModel,
    MessageGroup, OutgoingMessage,
};
pub use confirmation::{
    Answer, CARD_FIELD, CORE_LOCALPART, CardAudience, CardChoice, CardLine, ConfirmationAnswer,
    ConfirmationCard, ConfirmationList, ConfirmationReply, ConfirmationState, ConfirmationView,
    ConfirmationWindow, WindowEnd, WindowView, answer_path, window_path,
};
pub use confirmation_origin::{
    CONFIRMATIONS_PATH, CONFIRMATIONS_WAIT_SECS, ConfirmationOrigin, ConfirmationRequester,
    ConfirmationsViewModel, DeviceConfirmationPush, ORIGIN_META_KEY, ORIGIN_ROOM_HEADER,
    ORIGIN_THREAD_HEADER, confirmation_path,
};
pub use core_upgrade::{
    CORE_UPGRADE_CHECK_PATH, CORE_UPGRADE_PATH, CoreUpgradeBreakingPlugin,
    CoreUpgradeBreakingReason, CoreUpgradeCheck, CoreUpgradeCheckRequest, CoreUpgradeCompat,
    CoreUpgradeCompatCategory, CoreUpgradeCompatDetail, CoreUpgradeFailure, CoreUpgradeOffer,
    CoreUpgradePhase, CoreUpgradeProblem, CoreUpgradeProgress, CoreUpgradeRequest, CoreUpgradeRun,
    CoreUpgradeRunState, CoreUpgradeStage, CoreUpgradeStatus, CoreUpgradeTarget,
    CoreUpgradeViewModel, CoreVersion,
};
pub use egress::{
    EgressConfirmation, EgressCurrent, EgressFailure, EgressPathState, EgressProbe, EgressProblem,
    EgressProxyProvider, EgressProxySettings, EgressProxySource, EgressProxyUpdate, EgressRestart,
    EgressViewModel, LanWebuiUpdate,
};
pub use error::{CoreError, set_core_error_formatter};
pub use family::{
    BotKeeper, BotRef, FAMILY_ROOM_EVENT, FamilyAccess, GATEWAY_REFUSAL_HEADER, GatewayRefusal,
    ME_PATH, MEMBER_PAIRING_CODE_TTL_SECS, MEMBERS_PATH, Me, Member, MemberInvite, MemberList,
    MemberPairingCode, MemberRemoval, MemberSettings, MemberStatus, NewSharedBot, Role,
    SHARED_BOTS_PATH, TOOL_CALL_USER_META, member_pairing_code_path, member_path,
    member_removal_path, plugin_family_path,
};
pub use file::{
    FILE_ERROR_HEADER, FILES_HEADER, FileCard, FileCardAction, FileCardActions, FileError,
    FileErrorBody, FileFailure, FileGrantRequest, FileId, FileInfo, FileLinks, FileOperation,
    FileProblem, FileSource, InvalidFileId, RoomTransfers, Transfer, TransferAction,
    TransferDirection, TransferElsewhere, TransferState, TransfersViewModel, UploadSource,
    extract_file_links, files_message_body,
};
pub use friends::{
    FRIEND_EVENT, FriendBlock, FriendBlockScope, FriendBlockTarget, FriendEvent, FriendFailure,
    FriendGroup, FriendProblem, FriendRequest, FriendRequestKind, FriendRow, FriendsList,
    FriendsState, FriendsViewModel, RemovedFriend,
};
pub use home::{
    ConnectionCheck, ConnectionDetails, ConnectionNotice, ConnectionQuality, DeviceInvite,
    DeviceNetwork, HomeFailure, HomePath, HomeStatus, HomeViewModel, PairingFailure, PairingStage,
    PairingStatus, ProxySettings,
};
pub use lan::{
    LanAddress, LanChoice, LanFailure, LanInterface, LanInterfaceKind, LanPlugin, LanPort,
    LanProblem, LanProtocol, LanSettings,
};
pub use link::AppLink;
pub use link_preview::{
    LINK_PREVIEW_PATH, LinkPreview, LinkPreviewFailure, LinkPreviewImage, link_preview_image_path,
};
pub use matrix_login::{
    MatrixAccount, MatrixLoginFailure, MatrixLoginMethod, MatrixLoginState, MatrixLoginViewModel,
    MatrixOidcCallback, MatrixOidcLogin, MatrixPasswordLogin,
};
pub use onboarding::{
    AgentLogin, CUSTOM_MODEL_PROVIDER_PATH, CUSTOM_MODEL_PROVIDER_TEST_PATH,
    ConfiguredModelProvider, CustomModelProvider, MODEL_PROVIDERS_PATH, ModelProvider,
    ModelProviderFailure, ModelProviderInput, ModelProviderKey, ModelProviderProblem,
    ModelProviders, ONBOARDING_PATH, Onboarding, OnboardingAgent, OnboardingAgentRow,
    OnboardingCommit, OnboardingItemProblem, OnboardingItemState, OnboardingModelChoice,
    OnboardingModelStep, OnboardingNeed, OnboardingNeedRow, OnboardingProfileStep,
    OnboardingReview, OnboardingReviewDetail, OnboardingReviewItem, OnboardingReviewKind,
    OnboardingStep, OnboardingTip, OnboardingTips, OnboardingTooOld, OnboardingUpdate,
    OnboardingViewModel, ReplacedAgent, StepNav, model_provider_path, model_provider_test_path,
};
pub use owner::{
    MAX_DISPLAY_NAME_CHARS, OwnerProfile, OwnerProfileUpdate, ProfileProblem, check_display_name,
    check_email, check_email_change, normalized_email,
};
pub use panel::{
    BOT_PANEL_PINS_PATH, BotPanelPin, BotPanelPins, BotPinConsent, BotPinOwner, LAYOUT_EVENT,
    LayoutSettings, MAX_PANEL_PAGES, MCP_APP_CALLS_PATH, MCP_APP_MESSAGE_PATH, MCP_APP_READ_PATH,
    MCP_APP_TOOLS_PATH, PanelEntry, PanelItem, PanelScope, PanelSource, PinTarget, PlacedTab,
    ROOM_PANEL_EVENT, RoomPanel, TabDisplaySetting, bot_panel_consent_path, bot_panel_pin_path,
    bot_panel_pin_seen_path, mcp_app_call_path, mcp_app_host_path,
};
pub use peers::{
    PEER_ERROR_HEADER, PEERS_PATH, PeerCode, PeerError, PeerErrorBody, PeerProblem, PeerRequestRow,
    PeerRow, PeerScan, PeerStatus, PeersState, PeersView, PeersViewModel, RejectedPeerRow,
    peer_code_path, peer_code_reset_path, peer_path, peer_rejected_path, peer_request_approve_path,
    peer_request_reject_path,
};
pub use plugin::LanClient;
pub use plugin::StoreUnreadable;
pub use plugin::hint::{PluginHint, PluginHintCategory};
pub use plugin::ports::{PortProtocol, PortSpec, TunnelSpec};
pub use plugin::store::{StoreConfirmation, UpdatablePlugin};
pub use plugin::{
    AddSourceRequest, AddedSource, AddedTools, AskEveryUpdate, BlockedCategory, BotSwitch,
    CapabilityDefaultChoice, CapabilityProvider, CapabilityProviderRef, CapabilityProviderSettings,
    CapabilitySetting, ConsumerCapabilityChoice, ConsumerCapabilitySetting, InUseError,
    InstallPreview, InstallProgress, InstallRequest, InstallSource, InstallStage, InstallTogether,
    InstalledPlugin, InstalledPlugins, LanWebuiOffer, MAX_PLUGIN_ARCHIVE_BYTES, MissingCapability,
    OpenWindow, OwnerPluginSecret, OwnerPluginSecretValue, OwnerPluginSecrets, OwnerSecretFailure,
    OwnerSecretProblem, OwnerSecretView, OwnerSecretsView, PermissionLine, PermissionOpened,
    PermissionRisk, PermissionUpdate, PluginAuthor, PluginDetail, PluginDevice, PluginFailure,
    PluginHealth, PluginIcon, PluginImage, PluginKind, PluginLimits, PluginOperation,
    PluginProblem, PluginRuntime, PluginScope, PluginSettingField, PluginSettingKind,
    PluginSettingOption, PluginSettingsUpdate, PluginSettingsView, PluginState, PluginTakeover,
    PluginTask, PluginTaskNextStep, PluginTaskState, PluginTrust, PluginWriter, PluginsViewModel,
    RequirementLine, RequirementStatus, SecretInput, SecretInputState, SecretUpdate, SecretValues,
    SecurityTier, SettingValue, SettingValues, SettingsFieldError, SettingsFieldProblem,
    SettingsFieldProblemKind, SettingsForm, SettingsInvalidError, SettingsProblem,
    SettingsSchemaError, SettingsSchemaProblem, SettingsSchemaProblemKind, SourceKind,
    StorageUsage, StoreFailure, StoreItem, StoreListing, StoreProblem, StoreSection, TakeoverState,
    ToolAccess, ToolPolicyEntry, ToolPolicyView, UninstallPreview, UninstallRequest, UpgradeOffer,
    UpgradeRequest, UpgradeResult, plugin_scope_path, plugin_settings_path,
    plugin_settings_preview_path, secret_input_path,
};
pub use plugin_mail::{MAIL_FIELD, PluginMail, PluginMailAttachment};
pub use plugin_notice::{NOTICE_FIELD, NoticeLevel, PluginNotice};
pub use port_mapping::{
    MappedPort, PluginPortMapping, PortMappingState, PortMappingWarning, UdpFlowStats,
};
pub use push::{
    NotificationCategory, PushConfirmation, PushContent, PushEndpoint, PushMethod,
    PushNotification, PushStatus,
};
pub use registry_mirror::{
    RegistryFailure, RegistryMirrorSettings, RegistryMirrorTest, RegistryMirrorTestRequest,
    RegistryMirrorTests, RegistryMirrorUpdate, RegistryMirrorViewModel, RegistryProbe,
    RegistryProblem,
};
pub use reply::{ComposerDraft, QuotedContent, QuotedMessage, ReplyQuote};
pub use roles::{
    AdminScope, AiLevel, AppScope, BOT_AI_LEVEL_ENV, CAPABILITIES_HEADER, Capability,
    DEFAULT_INVITE_ROLE_PATH, Gate, JOIN_REQUESTS_PATH, JoinOutcome, JoinRequest, JoinRequestList,
    OIDC_USER_GROUP, PERMISSION_REQUIRED, PermissionItem, PermissionRequired, Permissions,
    PluginAudience, ROLES_PATH, RoleAssignment, RoleDef, RoleDraft, RoleList, RoleTemplate,
    TOOL_CALL_CAPABILITIES_META, friend_role_path, join_request_approve_path,
    join_request_reject_path, member_role_path, plugin_audience_path, role_path,
};
pub use secret::SecretString;
pub use server_version::{is_older, minimum_server_version};
pub use servers::{ServerKind, ServerName, ServerRow, ServersViewModel};
pub use session::{SessionListViewModel, SessionRow};
pub use settings::{SettingsEntry, SettingsSection, SettingsViewModel};
pub use share_transport::{
    MatrixOpenIdToken, SHARE_ALPN, ShareReply, ShareRequest, SharedPlugins, WONTON_WELL_KNOWN_PATH,
    WontonDiscovery,
};
pub use sharing::{
    AppsAction, PluginShares, ShareFailure, ShareGrant, ShareGrantUpdate, ShareIdentityFailure,
    ShareProblem, ShareRecipient, SharedAppRow, SharedAppsSource, SharedAppsState,
    SharedAppsViewModel, SharedPlugin,
};
pub use sidebar::{SidebarApp, SidebarOpen, SidebarViewModel};
pub use tabs::{
    CandidatePlugin, PluginTab, TabCandidate, TabDisplay, TabDisplayChoice, TabIcon, TabItem,
    TabLayout, TabLoadFailure, TabPage, TabRef, TabState, TabsProblem, TabsViewModel,
};
pub use timeline::{
    BackPagination, ImageCaption, ImageContent, KeyNotice, MatrixTarget, MediaHandle,
    MessageAction, MessageContent, RichBlock, RichBlockKind, RichLink, RichLinkKind, RichSpan,
    SendState, TimelineContent, TimelineDiff, TimelineItem, TimelineMessage, TimelineScroll,
    TimelineUpdate, TimelineViewport,
};
pub use verification::{
    KeyHandoff, VerificationCancel, VerificationEmoji, VerificationFlow, VerificationStage,
};
