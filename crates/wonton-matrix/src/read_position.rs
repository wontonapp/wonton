//! 本账号在一个房间的已读位置和未读条数：进房间停在哪、按钮上写几，都按它算（ADR-0071）。

use matrix_sdk::ruma::{EventId, OwnedEventId};
use tokio::sync::broadcast;

use crate::ChatRoom;

impl ChatRoom {
    /// 本账号在这个房间的未读条数，和聊天列表那一行是同一个数。
    pub fn unread_messages(&self) -> u64 {
        self.room.num_unread_messages()
    }

    /// 房间信息（未读数、已读回执）变了时醒一次。所有房间的更新都送到这里，按房间 ID 过滤。
    pub fn subscribe_to_info_updates(
        &self,
    ) -> broadcast::Receiver<matrix_sdk_base::RoomInfoNotableUpdate> {
        self.room.client().room_info_notable_update_receiver()
    }

    /// 本账号的已读位置：最近一条已读回执指着的事件。从没读过时为空；回执指着的事件可能已经不在时间线里。
    pub async fn read_position(&self) -> Option<OwnedEventId> {
        let own = self.room.own_user_id().to_owned();
        self.timeline()
            .await
            .latest_user_read_receipt_timeline_event_id(&own)
            .await
    }
}

/// 事件 ID 对应的条目 ID。SDK 给每个事件条目生成的 `unique_id` 就是 `__eid_` 加事件 ID；前缀是 SDK 的内部约定，
/// 对不上时只会让已读位置在条目里找不到，落点退回按未读条数往回数。
pub fn entry_id(event_id: &EventId) -> String {
    format!("__eid_{event_id}")
}
