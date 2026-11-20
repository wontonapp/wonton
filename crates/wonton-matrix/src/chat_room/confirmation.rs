//! 等一条已经放进发送队列的消息有结果：服务器收下（拿到事件 ID），或者不可恢复地失败。
//!
//! 转发附言靠它：附言发出去了才发转发，附言被拒就不转发。队列本身对同一房间是先进先出，不可恢复的失败还会挡住
//! 之后的消息（`vendor/matrix-sdk/src/send_queue/mod.rs` 的 `sending_task`）；但转发只排进队列的话，附言被拒
//! 时它只是跟着卡在队首，界面上一直“发送中”。这里等出结果，让转发要么真的排在附言后面，要么当场报失败。

use matrix_sdk::send_queue::{RoomSendQueueUpdate, SendHandle, SendQueueUpdate};
use tokio::sync::broadcast::Receiver;
use tokio::sync::broadcast::error::RecvError;

use super::ChatRoom;

/// 一条消息的结果。
#[derive(Clone, Debug)]
pub enum SendOutcome {
    /// 服务器收下了。
    Sent,
    /// 发不出去了，附带原因。
    Failed(String),
}

impl ChatRoom {
    /// 订阅发送结果。要在把消息放进队列**之前**订阅，否则可能错过它的结果。
    pub fn send_updates(&self) -> Receiver<SendQueueUpdate> {
        self.room.client().send_queue().subscribe()
    }

    /// 等 `handle` 指的那条消息有结果，`updates` 是发之前从 [`Self::send_updates`] 拿到的。可恢复的错误
    /// （连不上、服务器暂时出错）不算结果：队列会自己重发。
    pub async fn wait_outcome(
        &self,
        mut updates: Receiver<SendQueueUpdate>,
        handle: SendHandle,
    ) -> SendOutcome {
        let room_id = self.room_id();
        let transaction_id = handle.transaction_id().to_owned();
        loop {
            match updates.recv().await {
                Ok(SendQueueUpdate {
                    room_id: updated,
                    update,
                }) if updated == room_id => match update {
                    RoomSendQueueUpdate::SentEvent {
                        transaction_id: sent,
                        ..
                    } if sent == transaction_id => return SendOutcome::Sent,
                    RoomSendQueueUpdate::SendError {
                        transaction_id: failed,
                        error,
                        is_recoverable: false,
                    } if failed == transaction_id => {
                        return SendOutcome::Failed(error.to_string());
                    }
                    RoomSendQueueUpdate::CancelledLocalEvent {
                        transaction_id: cancelled,
                    } if cancelled == transaction_id => {
                        return SendOutcome::Failed("这条消息被取消了".to_owned());
                    }
                    // 可恢复的错误留着等重发；别的房间、别的条目不关心。
                    _ => {}
                },
                Ok(_) => {}
                // 漏掉的更新可能包含这条的结果，再等也没有；问时间线里的那条现在是什么状态。
                Err(RecvError::Lagged(_)) => {
                    if let Some(outcome) = self.current_outcome(&transaction_id).await {
                        return outcome;
                    }
                }
                Err(RecvError::Closed) => {
                    return SendOutcome::Failed("发送队列停了".to_owned());
                }
            }
        }
    }

    /// 订阅漏掉更新时按时间线里的状态补一次；还没到出结果的状态时返回 None。
    async fn current_outcome(
        &self,
        transaction_id: &matrix_sdk::ruma::OwnedTransactionId,
    ) -> Option<SendOutcome> {
        use matrix_sdk_ui::timeline::EventSendState;
        let timeline = self.timeline().await;
        for item in timeline.items().await.iter() {
            let Some(event) = item.as_event() else {
                continue;
            };
            let Some(handle) = event.local_echo_send_handle() else {
                continue;
            };
            if handle.transaction_id().as_str() != transaction_id.as_str() {
                continue;
            }
            return match event.send_state() {
                Some(EventSendState::Sent { .. }) => Some(SendOutcome::Sent),
                Some(EventSendState::SendingFailed {
                    is_recoverable: false,
                    error,
                }) => Some(SendOutcome::Failed(error.to_string())),
                // 还在排队、重发，或者刚漏掉更新还没来得及反映过来：接着等新的更新。
                _ => None,
            };
        }
        None
    }
}
