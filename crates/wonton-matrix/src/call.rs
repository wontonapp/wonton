//! 房间里的 MatrixRTC 通话（ADR-0096）：有没有人在通话按房间的 `m.call.member` 状态算。

use crate::ChatRoom;

impl ChatRoom {
    /// 房间里有没过期的通话成员（MSC4143 的 `m.call` 应用、房间范围），自己别的设备也算。
    /// 状态跟着同步变，变了 [`ChatRoom::subscribe_to_info_updates`] 会醒；过期只按时间算，不会单独醒。
    pub fn call_active(&self) -> bool {
        self.room.has_active_room_call()
    }
}
