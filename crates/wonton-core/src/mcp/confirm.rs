//! 敏感操作的确认（ADR-0016）：核心发卡片、等主人回答、验证回答，Agent 只能发起，没法替主人同意。
//!
//! 主人的回答只经 [`Confirmations::answer`] 进来，网关只在已配对设备或主人令牌的请求上调用它（ADR-0024）。

mod activity;
mod answer;
mod ask;
mod settle;
mod store;
mod types;
mod view;
mod windows;

pub use windows::EndedWindow;

#[allow(unused_imports)] // 只有测试经 `super::confirm::DEFAULT_WINDOW` 用。
pub use store::{Confirmations, DEFAULT_WINDOW};
pub use types::{AnswerError, Audience, Chosen, Notifier, Recipients, Refusal, Ticket};

pub(super) use store::{Slot, Table, now_ms};
pub(super) use types::{CANCELLED, SUPERSEDED};
pub(super) use wonton_protocol::ConfirmationState;
