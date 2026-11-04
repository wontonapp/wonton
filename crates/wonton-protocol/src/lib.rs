//! 核心 API、ViewModel 与插件清单（`x-wonton`）的类型定义，是服务端、客户端和插件之间的唯一事实源。
//!
//! 修改这里的公开类型属于跨模块接口变更：先单独提 PR 合并，再由依赖方跟进（见 AGENTS.md）。
//!
//! 所有类型都可以用 serde 序列化，供测试和将来的 Web 端使用；启用 `uniffi` feature 后，
//! 同一份类型经 UniFFI 生成 Swift / Kotlin 类型，原生壳不再手写一份。

#[cfg(feature = "uniffi")]
uniffi::setup_scaffolding!();

mod action;
mod appearance;
mod chat;
mod confirmation;
mod core_upgrade;
mod egress;
mod error;
mod file;
mod home;
mod lan;
mod plugin;
mod push;
mod secret;
mod session;
mod timeline;

pub use action::Action;
pub use appearance::{
    AppearanceProblem, AppearanceViewModel, FontOption, FontStyle, Rgb, TextSize, TextSizeOption,
    ThemeColors, ThemeMode, ThemeModeOption,
};
pub use chat::{
    ChatAccount, ChatFailure, ChatProblem, ChatSync, ChatViewModel, DeviceVerification,
    EncryptionStatus, ImageThumbnail, ImageUpload, PendingConfirmation, RecoveryStatus,
    RoomMembership, RoomPreview, RoomRow,
};
pub use confirmation::{
    Answer, CARD_FIELD, CORE_LOCALPART, CardChoice, CardLine, ConfirmationAnswer, ConfirmationCard,
    ConfirmationList, ConfirmationReply, ConfirmationState, ConfirmationView, answer_path,
};
pub use core_upgrade::{
    CORE_UPGRADE_CHECK_PATH, CORE_UPGRADE_PATH, CoreUpgradeCheck, CoreUpgradeCheckRequest,
    CoreUpgradeFailure, CoreUpgradeOffer, CoreUpgradePhase, CoreUpgradeProblem,
    CoreUpgradeProgress, CoreUpgradeRequest, CoreUpgradeRun, CoreUpgradeRunState, CoreUpgradeStage,
    CoreUpgradeStatus, CoreUpgradeTarget, CoreUpgradeViewModel, CoreVersion,
};
pub use egress::{
    EgressConfirmation, EgressCurrent, EgressFailure, EgressPathState, EgressProbe, EgressProblem,
    EgressProxySettings, EgressProxySource, EgressProxyUpdate, EgressRestart, EgressViewModel,
    LanWebuiUpdate,
};
pub use error::CoreError;
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
pub use plugin::{
    AddSourceRequest, AddedSource, InstallPreview, InstallRequest, InstallSource, InstalledPlugin,
    InstalledPlugins, LanWebuiOffer, MAX_PLUGIN_ARCHIVE_BYTES, PermissionLine, PluginAuthor,
    PluginDetail, PluginFailure, PluginHealth, PluginIcon, PluginImage, PluginKind,
    PluginOperation, PluginProblem, PluginState, PluginTask, PluginTaskState, PluginTrust,
    PluginsViewModel, RequirementLine, RequirementStatus, SourceKind, StorageUsage, StoreFailure,
    StoreItem, StoreListing, StoreProblem, StoreSection, UpgradeOffer, UpgradeRequest,
};
pub use push::{PushConfirmation, PushContent, PushEndpoint, PushNotification, PushStatus};
pub use secret::SecretString;
pub use session::{SessionListViewModel, SessionRow};
pub use timeline::{
    BackPagination, ImageContent, MediaHandle, MessageAction, MessageContent, RichBlock,
    RichBlockKind, RichSpan, SendState, TimelineContent, TimelineDiff, TimelineItem,
    TimelineMessage, TimelineUpdate,
};
