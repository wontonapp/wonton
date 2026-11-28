use std::sync::Arc;

use tokio::runtime::Handle;
use wonton_matrix::{ChatRoom, OwnedRoomId};

use super::Jumps;

/// 转发循环要用的、按订阅建一次的状态。
pub(in crate::chat) struct Live {
    pub(in crate::chat) typing: Arc<super::super::room_typing::Typing>,
    pub(in crate::chat) tables: Arc<super::super::room_commands::Tables>,
    pub(in crate::chat) jumps: Arc<Jumps>,
    /// 本账号的“正在输入”节流。
    pub(in crate::chat) typing_out: Arc<super::super::out_typing::TypingOut>,
    /// 这条订阅的编辑态。
    pub(in crate::chat) edit: Arc<super::super::edit::Edit>,
}

/// 测试用：不起订阅和任务的状态。
#[cfg(test)]
pub(in crate::chat) fn seeded() -> Arc<Live> {
    Arc::new(Live {
        typing: super::super::room_typing::Typing::seeded(&[]),
        tables: super::super::room_commands::Tables::new(
            OwnedRoomId::try_from("!room:home.example").unwrap(),
        ),
        jumps: Jumps::new(),
        typing_out: super::super::out_typing::seeded(),
        edit: Arc::new(super::super::edit::Edit::default()),
    })
}

/// 一个订阅的状态：转发任务和壳的操作共用同一份。
pub(in crate::chat) fn live(
    runtime: &Handle,
    room: &Arc<ChatRoom>,
    room_id: OwnedRoomId,
) -> Arc<Live> {
    Arc::new(Live {
        typing: super::super::room_typing::Typing::spawn(runtime, room),
        tables: super::super::room_commands::Tables::new(room_id),
        jumps: Jumps::new(),
        typing_out: super::super::out_typing::TypingOut::new(runtime, room),
        edit: Arc::new(super::super::edit::Edit::default()),
    })
}
