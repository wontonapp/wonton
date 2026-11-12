//! Matrix 客户端封装，基于 matrix-rust-sdk，供客户端核心使用。
//!
//! 负责登录、Sliding Sync 同步、收发消息与附件。设计见 `docs/adr/0003-matrix-and-encrypted-push.md`
//! 与 `docs/adr/0009-identity-pairing-sso.md`。
//!
//! - [`MatrixSession`]：用核心签发的 JWT 登录 Tuwunel，或者用存下来的登录状态恢复；用 Simplified Sliding
//!   Sync（MSC4186）同步；建私聊、加入和离开房间、观察房间列表与资料变化；端到端加密的恢复；上传和下载媒体。
//! - [`ChatRoom`]：打开一个房间，收发文字和图片，回复、转发、撤回消息，按增量观察时间线，向前翻页，发已读回执。
//! - 推送：注册指向家里推送代理的 pusher（[`PusherConfig`]），推送到了取回那条消息并在本机解密
//!   （[`MatrixSession::notification`]）。
//!
//! 所有错误都按原因区分，见 [`Error`]。

// matrix-sdk 的 future 嵌套很深，默认的 128 层不够算出 `room.timeline()` 的类型布局。
#![recursion_limit = "256"]

mod agent_run;
mod chat_room;
mod encryption;
mod error;
mod friends;
mod group;
mod layout;
mod login;
mod media;
mod message;
mod panels;
pub mod platform_tls;
mod profile;
mod push;
mod read_position;
mod reply;
mod rooms;
mod saved;
mod send_queue;
mod session;
mod sharing;
mod side_chat;
mod store_holder;
mod sync;
#[cfg(feature = "testkit")]
pub mod testkit;
mod timeline;
mod verification;

pub use agent_run::AgentRunStatus;
pub use chat_room::{
    BackPaginationStatus, ChatRoom, ImageThumbnail, ImageUpload, MISSED_GRACE, MissedEvent,
};
pub use encryption::{EncryptionState, Recovery, Verification};
pub use error::{Error, Result};
pub use friends::{Blocked, FRIEND_CREATE_KEY, FriendChat, FriendInvite};
pub use group::{GroupPart, UploadedImage};
pub use layout::{CHAT_LAYOUT_EVENT, ChatLayoutContent};
pub use login::{LoginError, OidcClient, OidcStart, PendingLogin};
pub use matrix_sdk::ruma::{
    DeviceId, EventId, OwnedDeviceId, OwnedEventId, OwnedRoomId, OwnedServerName, OwnedUserId,
    RoomId, ServerName, UserId, events::relation::RelationType,
};
pub use matrix_sdk_ui::eyeball_im::{Vector, VectorDiff};
pub use media::{PreviewImageSource, encrypted_media_token, media_token};
pub use message::{FileContent, ImageContent, Message, MessageBody, SendStatus, UploadProgress};
pub use panels::{LAYOUT_EVENT, LayoutContent, ROOM_PANEL_EVENT, RoomPanelContent};
pub use profile::UserProfile;
pub use push::{BackgroundNotification, Notification, PusherConfig};
pub use read_position::entry_id;
pub use reply::{Draft, Quoted, RepliedMessage, ReplyTo};
pub use rooms::{LatestMessage, Membership, RoomInfo};
pub use session::{MatrixSession, SavedSession, SessionConfig, SessionEvent, StoreConfig};
pub use sharing::{FriendSource, OpenIdToken};
pub use side_chat::{
    LatestSideChat, ResultLine, SIDE_CHAT_FIELD, SIDE_CHAT_RESULT_FIELD, SIDE_CHAT_SUPPORT_EVENT,
    SideChatFacts, ThreadActivity, result_line, root_marker, title_from_body,
};
pub use sync::{SyncStatus, last_sync_diagnostic};
pub use timeline::{EntryKind, TimelineEntry};
pub use verification::SelfVerification;
