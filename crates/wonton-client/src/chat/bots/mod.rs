//! Bot 名单：本人 Bot 的身份、核心给的 Bot 可达清单、资料头像和活动状态。

mod activity;
mod avatar;
mod contacts;
mod own;
mod profile;
mod shared;
mod state;

pub use avatar::bot_current_avatar;
#[cfg(test)]
pub(in crate::chat) use own::OwnBotFetch;
pub(in crate::chat) use state::{Bots, Local, Record};
