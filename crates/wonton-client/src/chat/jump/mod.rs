//! 跳到被回复的那条、再按原路回来（ADR-0109 §4）；未读胶囊跳回最早一条（ADR-0121）：返回栈、找不到时的往前翻页，
//! 和滚到底清栈。

mod handle;
mod live;
mod paging;
mod requests;
mod resolve;
mod state;
mod subscription;

#[cfg(test)]
pub(super) use live::seeded;
pub(super) use live::{Live, live};
pub(super) use state::Jumps;

#[cfg(test)]
use paging::{page_limit_reached, unavailable_reason};
#[cfg(test)]
use resolve::{Target, resolve};
#[cfg(test)]
use state::Wanted;
#[cfg(test)]
use std::collections::HashSet;
#[cfg(test)]
use wonton_protocol::UnreadJumpUnavailable;

#[cfg(test)]
#[path = "jump_tests.rs"]
mod jump_tests;
