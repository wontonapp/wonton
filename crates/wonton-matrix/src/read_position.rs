//! 本账号在一个房间的已读位置和未读条数：进房间停在哪、按钮上写几，都按它算（ADR-0071）。

use matrix_sdk::ruma::{EventId, OwnedEventId};
use tokio::sync::broadcast;

use crate::ChatRoom;

impl ChatRoom {
    /// 本账号在这条时间线上的未读条数：主对话按 `main` 回执算（话题回复不算），话题按它自己的话题回执算
    /// （ADR-0093 §4）。
    pub async fn unread_messages(&self) -> u64 {
        match self.thread_root() {
            Some(root) => self.thread_activity(root).await.unread,
            None => self.room.num_unread_messages(),
        }
    }

    /// 本账号还没读的 @ 条数（ADR-0121 的胶囊“有人@你”状态）；话题里也给房间的合计，够用来决定要不要提醒。
    pub fn unread_mentions(&self) -> u64 {
        self.room.num_unread_mentions()
    }
    /// 房间信息（未读数、已读回执）变了时醒一次。所有房间的更新都送到这里，按房间 ID 过滤。
    pub fn subscribe_to_info_updates(
        &self,
    ) -> broadcast::Receiver<matrix_sdk_base::RoomInfoNotableUpdate> {
        self.room.client().room_info_notable_update_receiver()
    }

    /// 本账号的已读位置：`m.fully_read` 和最近一条回执里更晚的一个（ADR-0121）。从没读过时为空；
    /// 位置可能已经不在时间线里（表情、话题回复），在下游按时间戳兜底。
    ///
    /// 旁聊用话题自己的回执：`m.fully_read` 不分话题（ADR-0093 §4）。
    pub async fn read_position(&self) -> Option<OwnedEventId> {
        let own = self.room.own_user_id().to_owned();
        let receipt = self
            .timeline()
            .await
            .latest_user_read_receipt_timeline_event_id(&own)
            .await;
        let fully_read = self
            .thread_root()
            .is_none()
            .then(|| self.room.fully_read_event_id())
            .flatten();
        match (receipt, fully_read) {
            (Some(receipt), Some(fully_read)) => {
                let items = self.timeline().await.items().await;
                let timestamp = |event_id: &EventId| {
                    items.iter().find_map(|item| {
                        let event = item.as_event()?;
                        (event.event_id() == Some(event_id)).then(|| event.timestamp())
                    })
                };
                match (timestamp(&receipt), timestamp(&fully_read)) {
                    // 两个都在时间线里：按位置比。
                    (Some(receipt_at), Some(fully_read_at)) => {
                        if fully_read_at >= receipt_at {
                            Some(fully_read)
                        } else {
                            Some(receipt)
                        }
                    }
                    // 只有一个在时间线里：它更晚。
                    (_receipt_at, None) => Some(receipt),
                    (None, _fully_read_at) => Some(fully_read),
                }
            }
            (receipt, fully_read) => receipt.or(fully_read),
        }
    }
}

/// 事件 ID 对应的条目 ID。SDK 给每个事件条目生成的 `unique_id` 就是 `__eid_` 加事件 ID；前缀是 SDK 的内部约定，
/// 对不上时只会让已读位置在条目里找不到，落点退回按未读条数往回数。
pub fn entry_id(event_id: &EventId) -> String {
    format!("__eid_{event_id}")
}
