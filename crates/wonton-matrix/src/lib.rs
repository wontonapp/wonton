//! Matrix 客户端封装，基于 matrix-rust-sdk，供客户端核心使用。
//!
//! 负责登录、Sliding Sync 同步、收发消息与附件。设计见 `docs/adr/0003-matrix-and-encrypted-push.md`
//! 与 `docs/adr/0009-identity-pairing-sso.md`。
//!
//! - [`MatrixSession`]：用核心签发的 JWT 登录 Tuwunel，或者用存下来的登录状态恢复；用 Simplified Sliding
//!   Sync（MSC4186）同步；建私聊、加入和离开房间、观察房间列表；端到端加密的恢复；下载媒体。
//! - [`ChatRoom`]：打开一个房间，收发文字和图片，按增量观察时间线，向前翻页，发已读回执。
//! - 推送：注册指向家里推送代理的 pusher（[`PusherConfig`]），推送到了取回那条消息并在本机解密
//!   （[`MatrixSession::notification`]）。
//!
//! 所有错误都按原因区分，见 [`Error`]。

// matrix-sdk 的 future 嵌套很深，默认的 128 层不够算出 `room.timeline()` 的类型布局。
#![recursion_limit = "256"]

mod chat_room;
mod encryption;
mod error;
mod media;
mod message;
mod push;
mod rooms;
mod send_queue;
mod session;
mod sync;
#[cfg(feature = "testkit")]
pub mod testkit;
mod timeline;
mod verification;

pub use chat_room::{BackPaginationStatus, ChatRoom, ImageThumbnail, ImageUpload};
pub use encryption::{EncryptionState, Recovery, Verification};
pub use error::{Error, Result};
pub use matrix_sdk::ruma::{
    DeviceId, EventId, OwnedDeviceId, OwnedEventId, OwnedRoomId, OwnedUserId, RoomId, UserId,
};
pub use matrix_sdk_ui::eyeball_im::{Vector, VectorDiff};
pub use message::{ImageContent, Message, MessageBody, SendStatus, UploadProgress};
pub use push::{Notification, PusherConfig};
pub use rooms::{LatestMessage, Membership, RoomInfo};
pub use session::{MatrixSession, SavedSession, SessionConfig, SessionEvent, StoreConfig};
pub use sync::SyncStatus;
pub use timeline::{EntryKind, TimelineEntry};
pub use verification::SelfVerification;
