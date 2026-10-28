//! Matrix 客户端封装，基于 matrix-rust-sdk，供客户端核心使用。
//!
//! 负责登录、Sliding Sync 同步、收发消息与附件。设计见 `docs/adr/0003-matrix-and-encrypted-push.md`
//! 与 `docs/adr/0009-identity-pairing-sso.md`。
//!
//! - [`MatrixSession`]：用核心签发的 JWT 登录 Tuwunel，用 Simplified Sliding Sync（MSC4186）同步，
//!   建私聊、加入房间、观察房间列表。
//! - [`ChatRoom`]：打开一个房间，收发文字和图片，读出消息列表。
//!
//! 所有错误都按原因区分，见 [`Error`]。

// matrix-sdk 的 future 嵌套很深，默认的 128 层不够算出 `room.timeline()` 的类型布局。
#![recursion_limit = "256"]

mod chat_room;
mod error;
mod message;
mod room_list;
mod session;
mod sync;

pub use chat_room::{ChatRoom, ImageUpload};
pub use error::{Error, Result};
pub use matrix_sdk::ruma::{
    DeviceId, EventId, OwnedDeviceId, OwnedEventId, OwnedRoomId, OwnedUserId, RoomId, UserId,
};
pub use message::{ImageContent, Message, MessageBody, SendStatus};
pub use room_list::{Membership, RoomSummary};
pub use session::{MatrixSession, SessionConfig, SessionEvent, StoreConfig};
pub use sync::SyncStatus;
