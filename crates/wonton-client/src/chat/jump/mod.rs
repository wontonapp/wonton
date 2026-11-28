//! 跳到被回复的那条、再按原路回来（ADR-0109 §4）；未读胶囊跳回最早一条（ADR-0121）：返回栈、找不到时的往前翻页，
//! 和滚到底清栈。

mod handle;
mod paging;
mod requests;
mod resolve;
mod state;

use std::sync::Arc;

use tokio::runtime::Handle;
use wonton_matrix::{ChatRoom, OwnedRoomId};

use super::timeline::TimelineSubscription;
use super::util::lock;
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

/// 转发循环要用的、按订阅建一次的状态。
pub(super) struct Live {
    pub(super) typing: Arc<super::room_typing::Typing>,
    pub(super) tables: Arc<super::room_commands::Tables>,
    pub(super) jumps: Arc<Jumps>,
    /// 本账号的“正在输入”节流。
    pub(super) typing_out: Arc<super::out_typing::TypingOut>,
    /// 这条订阅的编辑态。
    pub(super) edit: Arc<super::edit::Edit>,
}

/// 测试用：不起订阅和任务的状态。
#[cfg(test)]
pub(super) fn seeded() -> Arc<Live> {
    Arc::new(Live {
        typing: super::room_typing::Typing::seeded(&[]),
        tables: super::room_commands::Tables::new(
            OwnedRoomId::try_from("!room:home.example").unwrap(),
        ),
        jumps: Jumps::new(),
        typing_out: super::out_typing::seeded(),
        edit: Arc::new(super::edit::Edit::default()),
    })
}

/// 一个订阅的状态：转发任务和壳的操作共用同一份。
pub(super) fn live(runtime: &Handle, room: &Arc<ChatRoom>, room_id: OwnedRoomId) -> Arc<Live> {
    Arc::new(Live {
        typing: super::room_typing::Typing::spawn(runtime, room),
        tables: super::room_commands::Tables::new(room_id),
        jumps: Jumps::new(),
        typing_out: super::out_typing::TypingOut::new(runtime, room),
        edit: Arc::new(super::edit::Edit::default()),
    })
}

impl TimelineSubscription {
    /// 跳到原消息：有就下一批给焦点，没有就往前翻页。
    pub(super) fn jump_to(&self, event_id: &str, from_item_id: &str, revision: u32) {
        if let Some(live) = lock(&self.live).as_ref() {
            live.jumps
                .request(event_id.to_owned(), from_item_id.to_owned(), revision);
        }
    }

    /// 胶囊：跳到最早未读（ADR-0121）。`position` 是读位置，`unread` 是房间未读数。
    pub(super) fn jump_to_first_unread(&self, position: Option<String>, unread: u32) {
        if let Some(live) = lock(&self.live).as_ref() {
            live.jumps.request_first_unread(position, unread);
        }
    }

    /// 本会话冻结的读位置条目 ID（ADR-0121）。
    pub(super) fn unread_read_item(&self) -> Option<String> {
        lock(&self.live)
            .as_ref()
            .and_then(|live| live.jumps.read_item())
    }

    /// 胶囊：跳到下一条未读 @（ADR-0121）。
    pub(super) fn jump_to_next_mention(&self, position: Option<String>, unread: u32) {
        if let Some(live) = lock(&self.live).as_ref() {
            live.jumps.request_next_mention(position, unread);
        }
    }

    /// 壳报告锚点进视野：收掉胶囊。
    pub(super) fn unread_anchor_seen(&self, item_id: String) {
        if let Some(live) = lock(&self.live).as_ref() {
            live.jumps.anchor_seen(item_id);
        }
    }
    /// 当前报过的跳转序号。
    pub(super) fn jumps_revision(&self) -> u32 {
        lock(&self.live)
            .as_ref()
            .map_or(0, |live| live.jumps.revision())
    }

    /// 壳报的跳转序号（[`wonton_protocol::TimelineAction::RegisterFocusRevision`]）。
    pub(super) fn register_focus_revision(&self, revision: u32) {
        if let Some(live) = lock(&self.live).as_ref() {
            live.jumps.register_revision(revision);
        }
    }

    /// 回到跳之前的那条；栈空了就回到底部。
    pub(super) fn jump_back(&self) {
        if let Some(live) = lock(&self.live).as_ref() {
            live.jumps.request_back();
        }
    }
}
