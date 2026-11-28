//! 消息：把 matrix-sdk-ui 时间线里的事件条目转成界面直接能用的结构。

mod choices;
mod content;
mod convert;
mod item;
mod names;

pub use content::{FileContent, ImageContent, MessageBody};
// `mentions_of` 只有本模块的测试在用，`crate::message::mentions_of` 这条路径留着。
#[allow(unused_imports)]
pub(crate) use convert::{body, mentions_of, message, msgtype_body, sender_name};
pub use item::{Message, OwnReaction, Reaction, SendStatus, UploadProgress};
pub(crate) use names::{MemberNames, RedactionPower, fallback_name};
