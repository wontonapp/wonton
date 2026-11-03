//! 发送出错以后接着发。
//!
//! matrix-sdk 的房间发送队列出了任何错都会停下，要嵌入方重新打开（`RoomSendQueue::set_enabled`），SDK 自己不会
//! （matrix-sdk 0.19.1 的实测见 #86）：
//! - 可恢复的错误（连不上、超时、服务器暂时出错）：那条留在队首，重新打开后再发，还不通就再报错、再停下。隔一段时间
//!   重新打开，连续出错时间隔加倍；这个房间发出去一条就从头算。调用方知道网络恢复时
//!   （[`MatrixSession::resume`](crate::MatrixSession::resume)）不等这个间隔，全部重新打开，间隔也从头算。
//! - 不可恢复的错误（服务器拒收）：那条标成卡住，存进本机的状态存储。队列只看队首，它挡住这个房间之后的所有消息，
//!   直到重发（[`ChatRoom::retry_send`](crate::ChatRoom::retry_send)）或取消
//!   （[`ChatRoom::cancel_send`](crate::ChatRoom::cancel_send)）。SDK 的重发、取消只唤醒队列，不重新打开它，所以
//!   这里马上重新打开：卡住的那条照样挡着，等主人处理。

use std::{collections::HashMap, future::Future, sync::Arc, time::Duration};

use futures_util::{FutureExt, StreamExt, stream::FuturesUnordered};
use matrix_sdk::{
    Client,
    ruma::{OwnedRoomId, RoomId},
    send_queue::{RoomSendQueueUpdate, SendQueueRoomError, SendQueueUpdate},
};
use tokio::sync::{Notify, broadcast::error::RecvError};

/// 可恢复的错误之后第一次重发前等多久。SDK 报错前已经重试过 3 次。
const FIRST_DELAY: Duration = Duration::from_secs(2);
/// 间隔最长这么久，连接恢复后最晚这么久发出去。
const MAX_DELAY: Duration = Duration::from_secs(30);

/// 照看 `client` 所有房间的发送队列，直到任务被中止（会话释放时）。
///
/// 先接着发上次没发完的：它们存在本机的状态存储里，SDK 要等那个房间的队列第一次打开（比如主人打开那个房间）才发。
pub(crate) async fn keep_sending(client: Client, resume: Arc<Notify>) {
    let queue = client.send_queue();
    // 先订阅再接着发，不漏掉它们的错误。
    let mut errors = queue.subscribe_errors();
    let mut updates = queue.subscribe();
    queue.respawn_tasks_for_rooms_with_unsent_requests().await;
    let mut delays = HashMap::new();
    let mut reopening = FuturesUnordered::new();
    loop {
        tokio::select! {
            error = errors.recv() => match error {
                Ok(SendQueueRoomError { room_id, is_recoverable: false, .. }) => {
                    reopen(&client, Some(&room_id)).await;
                }
                Ok(SendQueueRoomError { room_id, .. }) => {
                    let delay = next_delay(delays.get(&room_id).copied());
                    delays.insert(room_id.clone(), delay);
                    reopening.push(after(delay, Some(room_id)));
                }
                // 漏掉了一些错误，不知道是哪些房间的：过一会儿全部重新打开。
                Err(RecvError::Lagged(_)) => reopening.push(after(FIRST_DELAY, None)),
                Err(RecvError::Closed) => return,
            },
            update = updates.recv() => match update {
                Ok(SendQueueUpdate { room_id, update: RoomSendQueueUpdate::SentEvent { .. } }) => {
                    delays.remove(&room_id);
                }
                Ok(_) | Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => return,
            },
            Some(room_id) = reopening.next() => reopen(&client, room_id.as_deref()).await,
            () = resume.notified() => {
                delays.clear();
                reopening.clear();
                reopen(&client, None).await;
            }
        }
    }
}

fn next_delay(previous: Option<Duration>) -> Duration {
    previous.map_or(FIRST_DELAY, |delay| (delay * 2).min(MAX_DELAY))
}

fn after(
    delay: Duration,
    room_id: Option<OwnedRoomId>,
) -> impl Future<Output = Option<OwnedRoomId>> {
    tokio::time::sleep(delay).map(move |()| room_id)
}

/// 重新打开一个房间的发送队列；`room_id` 为空时打开全部房间的。
async fn reopen(client: &Client, room_id: Option<&RoomId>) {
    match room_id {
        Some(room_id) => {
            if let Some(room) = client.get_room(room_id) {
                room.send_queue().set_enabled(true);
            }
        }
        None => client.send_queue().set_enabled(true).await,
    }
}
