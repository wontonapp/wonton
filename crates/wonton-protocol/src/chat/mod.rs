//! 聊天：Matrix 账号、同步、端到端加密和房间列表（ADR-0003、ADR-0005）。
//!
//! 客户端核心经隧道把家里的 Tuwunel 当成标准的 Matrix 服务器用（ADR-0015）。这里的 ViewModel 是整份快照：
//! 来不及取走的中间状态会被合并，壳拿到后直接渲染。打开的房间的消息走增量，见 `timeline` 模块。

mod ask;
mod contacts;
mod encryption;
mod install;
mod media;
mod own_bot;
mod pending;
mod rooms;
mod session;
#[cfg(test)]
mod tests;
mod view;

pub use ask::{AskFailure, AskProblem, AskReceipt, AskState};
pub use contacts::ChatContact;
pub use encryption::{
    BackupReminder, DeviceVerification, EncryptionStatus, RecoveryNextStep, RecoveryStatus,
};
pub use install::{BotInstallRow, BotInstallState};
pub use media::{ImageThumbnail, ImageUpload};
pub use own_bot::{OwnBotFailure, OwnBotProblem, OwnPrivateBotView};
pub use pending::PendingConfirmation;
pub use rooms::{MessageSummary, RoomBot, RoomMembership, RoomPreview, RoomRow};
pub use session::{ChatAccount, ChatFailure, ChatProblem, ChatSync};
pub use view::ChatViewModel;
