//! 客户端厚核心：状态、业务逻辑、网络与 ViewModel，经 UniFFI 导出给 SwiftUI 与 Compose。
//!
//! 接口形态是“操作进、ViewModel 出”，并保持可序列化，以便测试和将来的 Web 端复用。
//! 设计见 `docs/adr/0005-native-clients-thick-core.md`。
//!
//! - 操作进：壳调用 [`ClientCore::dispatch`]，参数是 [`Action`]。
//! - ViewModel 出：壳调用 [`ClientCore::subscribe_session_list`]、[`ClientCore::subscribe_home`]、
//!   [`ClientCore::subscribe_chat`]，循环等待订阅的 `next()`，包装成 Swift 的 `AsyncSequence` 或 Kotlin 的
//!   `Flow`。打开的房间的时间线用 [`ClientCore::subscribe_timeline`]，推的是不能合并的增量批次。
//!
//! 和家里服务器的连接见 `home` 模块：iroh 隧道加 WebView 和 HTTP 客户端用的本地代理（ADR-0004）。聊天见
//! `chat` 模块：经隧道登录主人的 Matrix 账号（ADR-0003、ADR-0009）。会话列表还是 M0 验证 UniFFI 用的示例，
//! 只放在内存里。

// matrix-sdk 的 future 嵌套很深，默认的 128 层不够算出聊天后台任务的类型布局。
#![recursion_limit = "256"]

uniffi::setup_scaffolding!();

mod chat;
mod client;
mod home;
mod logging;
mod session_store;
mod subscription;

pub use chat::TimelineSubscription;
pub use client::ClientCore;
pub use home::Network;
pub use logging::init_logging;
pub use subscription::{ChatSubscription, HomeSubscription, SessionListSubscription};
pub use wonton_protocol::{
    Action, BackPagination, ChatAccount, ChatFailure, ChatProblem, ChatSync, ChatViewModel,
    CoreError, DeviceVerification, EncryptionStatus, HomeFailure, HomePath, HomeStatus,
    HomeViewModel, ImageContent, ImageThumbnail, ImageUpload, MediaHandle, MessageContent,
    ProxySettings, RecoveryStatus, RoomMembership, RoomPreview, RoomRow, SecretString, SendState,
    SessionListViewModel, SessionRow, TimelineContent, TimelineDiff, TimelineItem, TimelineMessage,
    TimelineUpdate,
};
