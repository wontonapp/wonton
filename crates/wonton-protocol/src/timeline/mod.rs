//! 打开的房间的时间线：按增量推送（ADR-0005 的验证结果）。
//!
//! 时间线可能有成百上千条，每次变化都推整份太浪费，所以推的是增量批次，不能合并：壳要按顺序把每一批应用到
//! 自己持有的列表上。第一批总是 [`TimelineDiff::Reset`]，给出当前的全部条目。条目和 matrix-rust-sdk 的时间线
//! 一一对应（含日期分隔线、已读标记等），下标就是 SDK 的下标；哪些事件进时间线由 `wonton-matrix` 的事件过滤决定。
//! 列表该停在哪也由核心说（[`TimelineUpdate::viewport`]、[`TimelineUpdate::anchor`]），壳只按这两个值画和滚。
//!
//! 还有解不开的消息时，每一批都带着该显示的加密入口（[`TimelineUpdate::key_notice`]，[`KeyNotice`]）：核心按账号的
//! 恢复状态决定是输恢复密钥还是请别的设备确认，加密状态变了就补发一批空的。壳只按类别写文字和按钮，不自己判断。

mod content;
mod diff;
mod media;
mod message;
mod position;
mod rich;
#[cfg(test)]
mod tests;
mod unread;
mod update;

pub use content::TimelineContent;
pub use diff::{TimelineDiff, TimelineItem};
pub use media::{FileAttachment, ImageCaption, ImageContent, MediaHandle};
pub use message::{MessageAction, MessageContent, MessageReceipt, TimelineMessage};
pub use position::{BackPagination, TimelineScroll, TimelineViewport};
pub use rich::{MatrixTarget, RichBlock, RichBlockKind, RichLink, RichLinkKind, RichSpan};
pub use unread::{UnreadJump, UnreadJumpState, UnreadJumpUnavailable};
pub use update::{KeyNotice, TimelineUpdate};
