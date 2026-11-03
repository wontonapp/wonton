//! 核心 API、ViewModel 与插件清单（`x-wonton`）的类型定义，是服务端、客户端和插件之间的唯一事实源。
//!
//! 修改这里的公开类型属于跨模块接口变更：先单独提 PR 合并，再由依赖方跟进（见 AGENTS.md）。
//!
//! 所有类型都可以用 serde 序列化，供测试和将来的 Web 端使用；启用 `uniffi` feature 后，
//! 同一份类型经 UniFFI 生成 Swift / Kotlin 类型，原生壳不再手写一份。

#[cfg(feature = "uniffi")]
uniffi::setup_scaffolding!();

mod action;
mod chat;
mod confirmation;
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
pub use chat::{
    ChatAccount, ChatFailure, ChatProblem, ChatSync, ChatViewModel, DeviceVerification,
    EncryptionStatus, ImageThumbnail, ImageUpload, RecoveryStatus, RoomMembership, RoomPreview,
    RoomRow,
};
pub use confirmation::{
    Answer, CARD_FIELD, CORE_LOCALPART, CardChoice, CardLine, ConfirmationAnswer, ConfirmationCard,
    ConfirmationReply, ConfirmationState, ConfirmationView, answer_path,
};
pub use error::CoreError;
pub use file::{
    FILE_ERROR_HEADER, FILES_HEADER, FileCard, FileError, FileErrorBody, FileFailure,
    FileGrantRequest, FileId, FileInfo, FileLinks, FileSource, InvalidFileId, Transfer,
    TransferDirection, TransferState, extract_file_links,
};
pub use home::{
    DeviceInvite, HomeFailure, HomePath, HomeStatus, HomeViewModel, PairingFailure, PairingStatus,
    ProxySettings,
};
pub use lan::{
    LanAddress, LanChoice, LanFailure, LanInterface, LanInterfaceKind, LanPlugin, LanPort,
    LanProblem, LanProtocol, LanSettings,
};
pub use plugin::{
    InstallPreview, InstallRequest, InstallSource, InstalledPlugin, InstalledPlugins,
    PermissionLine, PluginAuthor, PluginCategory, PluginDetail, PluginFailure, PluginHealth,
    PluginIcon, PluginImage, PluginKind, PluginOperation, PluginProblem, PluginState, PluginTask,
    PluginTaskState, PluginTrust, PluginsViewModel, StoreFailure, StoreItem, StoreListing,
    StoreProblem, StoreSection, UpgradeOffer, UpgradeRequest,
};
pub use push::{PushContent, PushEndpoint, PushNotification, PushStatus};
pub use secret::SecretString;
pub use session::{SessionListViewModel, SessionRow};
pub use timeline::{
    BackPagination, ImageContent, MediaHandle, MessageContent, SendState, TimelineContent,
    TimelineDiff, TimelineItem, TimelineMessage, TimelineUpdate,
};
