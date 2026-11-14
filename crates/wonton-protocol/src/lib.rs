//! 核心 API、ViewModel 与插件清单（`x-wonton`）的类型定义，是服务端、客户端和插件之间的唯一事实源。
//!
//! 修改这里的公开类型属于跨模块接口变更：先单独提 PR 合并，再由依赖方跟进（见 AGENTS.md）。
//!
//! 所有类型都可以用 serde 序列化，供测试和将来的 Web 端使用；启用 `uniffi` feature 后，
//! 同一份类型经 UniFFI 生成 Swift / Kotlin 类型，原生壳不再手写一份。

#[cfg(feature = "uniffi")]
uniffi::setup_scaffolding!();

mod action;
mod agent_chain;
mod agent_handoff;
mod agent_outbox;
mod agent_permission;
mod agent_run;
mod app_lock;
mod appearance;
mod bot_commands;
mod bot_contacts;
mod bot_profile;
mod bot_web;
mod bot_web_registration;
mod chat;
mod chat_layout;
mod clients;
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
mod pages;
mod panel;
mod peers;
mod personal_data;
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
mod share_in;
mod share_transport;
mod sharing;
mod side_chat;
mod sidebar;
mod timeline;
mod timeline_action;
mod verification;
mod widget;

pub use action::Action;
pub use agent_chain::{
    ACP_CHAIN_META_KEY, AGENT_CHAINS_API_PATH, AGENT_CHAINS_PATH, AGENT_CHAINS_WAIT_SECS,
    AGENT_LIMITS_PATH, AGENT_OUTBOX_PATH, AGENT_OUTBOX_WAIT_SECS, AGENT_SETTINGS_PATH,
    AgentChainAction, AgentChainBot, AgentChainRow, AgentChainStatus, AgentChainsViewModel,
    AgentConcurrency, AgentSettingsViewModel, BOT_TOKEN_HEADER, ChainCarrier, ChainError,
    ChainLimitKind, ChainLimits, ChainRefusal, ChainSpend, ChainTurnGrant, ChainTurnReport,
    DEFAULT_CONSUMER_AGENT_CONCURRENCY, SESSION_HEADER, WONTON_BOT_HEADER, WONTON_CHAIN_HEADER,
    WONTON_MAX_DEPTH_HEADER, WONTON_MAX_ROUNDS_HEADER, WONTON_MAX_TOKENS_HEADER,
    WONTON_ON_BEHALF_OF_HEADER, WONTON_PLUGIN_HEADER, WONTON_USER_HEADER, agent_chain_stop_path,
    agent_consumer_concurrency_path,
};
pub use agent_handoff::{
    HAND_OFF_STATUS_TOOL, HAND_OFF_TOOL, HandOffRequest, HandOffResult, HandOffSession,
    HandOffState, HandOffStatusRequest, SEND_MESSAGE_TOOL, SendMessageRequest,
};
pub use agent_outbox::{OutboxFailure, OutboxItem, OutboxKind, OutboxReceipt, OutboxResult};
pub use agent_permission::{
    AGENT_PERMISSION_PATH, AGENT_PERMISSION_TTL_SECS, AgentPermissionKind, AgentPermissionOption,
    AgentPermissionOutcome, AgentPermissionRequest,
};
pub use agent_run::{AgentRunChoice, AgentRunEnd, AgentRunState};
pub use app_lock::{AppLockTimeout, AppLockViewModel};
pub use appearance::{
    AppearanceProblem, AppearanceViewModel, FontStyle, Rgb, TextSize, ThemeColors, ThemeMode,
};
pub use bot_commands::{
    BOT_COMMANDS_EVENT, BotCommand, BotCommands, MAX_BOT_COMMANDS, MAX_BOT_COMMANDS_BYTES,
    MAX_COMMAND_DESCRIPTION_CHARS, MAX_COMMAND_HINT_CHARS, MAX_COMMAND_NAME_CHARS,
};
pub use bot_contacts::{
    BOT_CONTACT_CREATE_KEY, BotContact, BotContactFailure, BotContacts, BotContactsState,
    BotContactsViewModel, CONTACT_ERROR_HEADER, CONTACT_EVENT, CONTACT_ROOM_TYPE, ContactError,
    ContactErrorBody, ContactEvent, ContactKind, ContactOutcome, ContactProblem, ContactRequest,
    ContactRequestPush, ContactState, PendingContactRequest, RemoteContact,
    bot_contact_approve_path, bot_contact_path, bot_contact_reject_path, bot_contacts_path,
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
pub use clients::{
    Ban, CLIENTS_ERROR_HEADER, CLIENTS_HEADER, CaaState, CertificateState, ClientAuthKind,
    ClientsErrorBody, ClientsFailure, ClientsPlugin, ClientsProblem, ClientsStatus,
    CredentialEntry, DnsProviderPreset, DomainSettings, EntryBlock, EntryKind, EntrySwitch,
    GUEST_LINKS_MAX, GUEST_TTL_DEFAULT, GUEST_TTL_MAX, GateState, GuestLink, IngressKind,
    IngressSettings, LanRecordState, NewGuestLinkRequest, NewPasswordRequest, PasswordSummary,
    PasswordValue, PendingChange, PendingChangeKind, PendingOp, ProbeResult, PublicAccess,
    PublicItem, PublicReach, PublicRuleMode, PublicRules, PublicSwitchState, PutDomainRequest,
    PutIngressRequest, RevokeDeviceRequest, SetEntryRequest, clients_ban_path, clients_domain_path,
    clients_entry_path, clients_guest_path, clients_guests_path, clients_ingress_path,
    clients_password_path, clients_passwords_path, clients_path, clients_public_access_path,
    clients_status_path, dns_providers_path,
};
pub use composer::{
    AttachmentFailure, AttachmentKind, AttachmentState, ComposerAttachment, ComposerViewModel,
    MessageGroup, OutgoingMessage,
};
pub use confirmation::{
    Answer, CARD_FIELD, CORE_LOCALPART, CardAudience, CardChoice, CardLine, ConfirmationAnswer,
    ConfirmationCard, ConfirmationCategory, ConfirmationList, ConfirmationReply, ConfirmationState,
    ConfirmationTool, ConfirmationToolImpact, ConfirmationView, ConfirmationWindow, WindowEnd,
    WindowView, answer_path, window_path,
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
pub use family::FAMILY_ROOM_SYNC_PATH;
pub use family::MEMBER_SYNC_PATH;
pub use family::{
    BotKeeper, BotRef, FAMILY_ROOM_EVENT, GATEWAY_REFUSAL_HEADER, GatewayRefusal,
    HOP_LIMIT_ERRCODE, ME_PATH, MEMBER_PAIRING_CODE_TTL_SECS, MEMBERS_PATH, Me, Member,
    MemberInvite, MemberList, MemberPairingCode, MemberRemoval, MemberStatus, NewSharedBot, Role,
    SHARED_BOTS_PATH, TOOL_CALL_USER_META, member_pairing_code_path, member_removal_path,
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
    FriendsState, FriendsViewModel, INVITE_FILTER_EVENT, RemovedFriend,
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
    LINK_PREVIEW_PATH, LinkPreview, LinkPreviewCard, LinkPreviewCardImage, LinkPreviewFailure,
    LinkPreviewImage, LinkPreviewReason, LinkPreviewState, link_preview_image_path,
};
pub use matrix_login::{
    MatrixAccount, MatrixLoginFailure, MatrixLoginMethod, MatrixLoginState, MatrixLoginViewModel,
    MatrixOidcCallback, MatrixOidcLogin, MatrixPasswordLogin,
};
pub use onboarding::{
    AgentLogin, MODEL_SETTINGS_PATH, ModelSettings, ModelSettingsFailure, ModelSettingsPage,
    ModelSettingsProblem, ONBOARDING_PATH, Onboarding, OnboardingAgent, OnboardingAgentRow,
    OnboardingCommit, OnboardingItemProblem, OnboardingItemState, OnboardingModelChoice,
    OnboardingModelStep, OnboardingNeed, OnboardingNeedRow, OnboardingProfileStep,
    OnboardingReview, OnboardingReviewDetail, OnboardingReviewItem, OnboardingReviewKind,
    OnboardingStep, OnboardingTip, OnboardingTips, OnboardingTooOld, OnboardingUpdate,
    OnboardingViewModel, ReplacedAgent, StepNav,
};
pub use owner::{
    MAX_DISPLAY_NAME_CHARS, OwnerProfile, OwnerProfileUpdate, ProfileProblem, check_display_name,
    check_email, check_email_change, normalized_email,
};
pub use pages::{
    CandidatePlugin, KEEP_ALIVE_PAGES, MyAppEntry, MyAppRef, OpenPage, PagesProblem,
    PagesViewModel, PluginTab, TabCandidate, TabIcon, TabLayout, TabLoadFailure, TabMediaPrompt,
    TabPage, TabRef, TabState,
};
pub use panel::{
    BOT_PANEL_PINS_PATH, BotPanelPin, BotPanelPins, BotPinConsent, BotPinOwner, LAYOUT_EVENT,
    LayoutSettings, MAX_PANEL_PAGES, MCP_APP_CALLS_PATH, MCP_APP_MESSAGE_PATH, MCP_APP_READ_PATH,
    MCP_APP_TOOLS_PATH, PanelEntry, PanelItem, PanelScope, PanelSource, PinTarget, PlacedTab,
    ROOM_PANEL_EVENT, RoomPanel, bot_panel_consent_path, bot_panel_pin_path,
    bot_panel_pin_seen_path, mcp_app_call_path, mcp_app_host_path,
};
pub use peers::{
    PEER_ERROR_HEADER, PEERS_PATH, PeerCode, PeerError, PeerErrorBody, PeerProblem, PeerRequestRow,
    PeerRow, PeerScan, PeerStatus, PeersState, PeersView, PeersViewModel, RejectedPeerRow,
    peer_code_path, peer_code_reset_path, peer_path, peer_rejected_path, peer_request_approve_path,
    peer_request_reject_path,
};
pub use personal_data::{
    CACHED_LOCATION_MAX_AGE_MS, CLEANUP_PATH, CleanupRequest, CleanupResult, DATA_PATH,
    DEFAULT_HEALTH_RETENTION_DAYS, DEFAULT_LOCATION_RETENTION_DAYS, DataQueryResult, EXPORT_PATH,
    ExportBundle, FAILURE_META, HEALTH_PATH, HEALTH_QUERY_TOOL, HEALTH_REFRESH_TOOL, HealthSample,
    HealthSamples, HealthValue, LOCATION_HISTORY_TOOL, LOCATION_NOW_TOOL, LOCATION_PATH,
    LOCATION_REQUEST_TIMEOUT_MS, LOCATION_REQUESTS_PER_HOUR, LocationPoint, LocationPointKind,
    LocationScope, OsPermissionState, PERSONAL_DATA_CAPABILITY, PERSONAL_DATA_STATUS_TOOL,
    PersonalDataBotGrant, PersonalDataCategory, PersonalDataFailure, PersonalDataProblem,
    PersonalDataRead, PersonalDataType, PersonalDataTypeRow, PersonalDataViewModel, QUERY_PATH,
    RetentionPolicy, RetentionSetting, SleepStage, WriteAccepted, data_path, query_path,
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
    InstallFailure, InstallPreview, InstallProgress, InstallRequest, InstallSource, InstallStage,
    InstallTogether, InstalledPlugin, InstalledPlugins, LanWebuiOffer, MAX_PLUGIN_ARCHIVE_BYTES,
    MissingCapability, OpenWindow, OwnerPluginSecret, OwnerPluginSecretValue, OwnerPluginSecrets,
    OwnerSecretFailure, OwnerSecretProblem, OwnerSecretView, OwnerSecretsView, PermissionLine,
    PermissionOpened, PermissionRisk, PermissionUpdate, PluginAuthor, PluginDetail,
    PluginDetailOutcome, PluginDetailRoute, PluginDevice, PluginDeviceKind, PluginFailure,
    PluginHealth, PluginIcon, PluginImage, PluginKind, PluginLimits, PluginMediaGrant,
    PluginOperation, PluginProblem, PluginRuntime, PluginScope, PluginSettingField,
    PluginSettingKind, PluginSettingOption, PluginSettingsUpdate, PluginSettingsView, PluginState,
    PluginTakeover, PluginTask, PluginTaskNextStep, PluginTaskState, PluginTrust, PluginWidget,
    PluginWriter, PluginsViewModel, RemoteAuth, RemoteAuthorization, RemoteMcpDetail,
    RemoteMcpPreview, RemoteOauthGrant, RemoteOauthStart, RequirementLine, RequirementStatus,
    RoomWidget, SecretInput, SecretInputState, SecretUpdate, SecretValues, SecurityTier,
    SettingValue, SettingValues, SettingsFieldError, SettingsFieldProblem,
    SettingsFieldProblemKind, SettingsForm, SettingsInvalidError, SettingsProblem,
    SettingsSchemaError, SettingsSchemaProblem, SettingsSchemaProblemKind, SourceKind,
    StorageUsage, StoreFailure, StoreItem, StoreListing, StoreProblem, StoreSection, TakeoverState,
    ToolAccess, ToolPolicyEntry, ToolPolicyView, UninstallPreview, UninstallRequest, UpgradeOffer,
    UpgradeRequest, UpgradeResult, WidgetCapability, WidgetCapabilityLevel, WidgetEntry,
    plugin_remote_oauth_finish_path, plugin_remote_oauth_start_path, plugin_scope_path,
    plugin_settings_path, plugin_settings_preview_path, remote_oauth_start_path, secret_input_path,
};
pub use plugin::{PluginDesiredState, PluginLifecycle, PluginRecovery, PluginRecoveryStage};
pub use plugin::{PluginStart, PluginStartState, RoomPlugin};
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
    PluginAudience, PluginAudienceChoice, PluginAudienceNotice, PluginAudienceView, ROLES_PATH,
    RoleAssignment, RoleDef, RoleDraft, RoleList, RoleTemplate, RolesViewModel,
    TOOL_CALL_CAPABILITIES_META, friend_role_path, join_request_approve_path,
    join_request_reject_path, member_role_path, plugin_audience_path, role_path,
};
pub use secret::SecretString;
pub use server_version::{is_older, minimum_server_version};
pub use servers::{ServerKind, ServerName, ServerRow, ServersViewModel};
pub use session::{SessionListViewModel, SessionRow};
pub use settings::{
    NotificationCategorySetting, SettingsEntry, SettingsSection, SettingsViewModel,
};
pub use share_in::{
    PendingShare, SHARE_INBOX_DIR, SHARE_MANIFEST_FILE, SHARE_MAX_ITEMS, SHARE_READY_FILE,
    ShareInDestinations, ShareInImage, ShareInItem, ShareInManifest, ShareInManifestError,
    ShareInManifestItem, ShareInViewModel, is_share_token,
};
pub use share_transport::{
    MatrixOpenIdToken, SHARE_ALPN, ShareReply, ShareRequest, SharedPlugins, WONTON_WELL_KNOWN_PATH,
    WontonDiscovery,
};
pub use sharing::{
    AppsAction, PluginShares, ShareFailure, ShareGrant, ShareGrantUpdate, ShareIdentityFailure,
    ShareProblem, ShareRecipient, SharedAppRow, SharedAppsSource, SharedAppsState,
    SharedAppsViewModel, SharedPlugin,
};
pub use side_chat::{
    SideChatCard, SideChatResult, SideChatResultState, SideChatRoot, SideChatState, SideChatSupport,
};
pub use sidebar::{SidebarApp, SidebarBadge, SidebarOpen, SidebarViewModel};
pub use timeline::{
    BackPagination, ImageCaption, ImageContent, KeyNotice, MatrixTarget, MediaHandle,
    MessageAction, MessageContent, MessageReceipt, RichBlock, RichBlockKind, RichLink,
    RichLinkKind, RichSpan, SendState, TimelineContent, TimelineDiff, TimelineItem,
    TimelineMessage, TimelineScroll, TimelineUpdate, TimelineViewport,
};
pub use timeline_action::{TimelineAction, TimelineTarget};
pub use verification::{
    KeyHandoff, VerificationCancel, VerificationEmoji, VerificationFlow, VerificationStage,
};
pub use widget::{
    WIDGET_MAX_ROOMS, WIDGET_SNAPSHOT_PATH, WIDGET_SNAPSHOT_VERSION, WidgetAccount, WidgetAgent,
    WidgetContact, WidgetPlugin, WidgetRoom, WidgetSnapshot, WidgetSnapshotRead,
};
