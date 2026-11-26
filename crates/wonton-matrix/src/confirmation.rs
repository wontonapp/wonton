//! 等一条自己发的消息有没有被服务器收下：只看这个房间的发送队列，按事务号匹配，不猜“上一条”。

use std::fmt;

use matrix_sdk::ruma::TransactionId;
use matrix_sdk::send_queue::{LocalEcho, LocalEchoContent, RoomSendQueueUpdate, SendHandle};
use tokio::sync::broadcast::{Receiver, error::RecvError};

use crate::chat_room::ChatRoom;
use crate::{Error, Result};

/// 一条消息的结果。可恢复的错误（断网、服务器暂时不可用）会一直等着，队列自己重试；只有不可恢复地失败、
/// 被取消或队列停了才返回 [`Self::Failed`]。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendOutcome {
    /// 服务器收下了。
    Sent,
    /// 不会再发了。
    Failed(SendFailure),
}

/// 消息没发出去、也不会再自动重试的原因。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendFailure {
    /// 在发出去之前被取消。
    Cancelled,
    /// 发送队列关掉了。
    QueueStopped,
    /// 连发送队列都没进去：订阅队列或入队时就失败了。
    NotQueued(String),
    /// 查不到它在队列里的状态。
    Unreadable(String),
    /// 发送请求不可恢复地失败（服务器拒收、设备不信任等）。
    Server(String),
}

impl SendFailure {
    /// 固定类别，给日志和给主人看的句子用；同一类别的说明只可能换措辞。
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Cancelled => "comment_cancelled",
            Self::QueueStopped => "comment_queue_stopped",
            Self::Unreadable(_) => "comment_unreadable",
            Self::NotQueued(_) => "comment_not_queued",
            Self::Server(_) => "comment_rejected",
        }
    }

    /// 服务器或队列给的原文。
    pub fn detail(&self) -> Option<&str> {
        match self {
            Self::Cancelled | Self::QueueStopped => None,
            Self::NotQueued(detail) | Self::Unreadable(detail) | Self::Server(detail) => {
                Some(detail)
            }
        }
    }
}

impl fmt::Display for SendFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.detail() {
            Some(detail) => write!(f, "{}：{detail}", self.kind()),
            None => f.write_str(self.kind()),
        }
    }
}

impl ChatRoom {
    /// 订阅这个房间的发送更新。返回时队列里已有的本地回显一并给出，订阅前就有动静的也能按事务号查到。
    pub async fn send_updates(&self) -> Result<(Vec<LocalEcho>, Receiver<RoomSendQueueUpdate>)> {
        self.room
            .send_queue()
            .subscribe()
            .await
            .map_err(|error| Error::Upstream(format!("订阅不了发送队列：{error}")))
    }

    /// 发一条文字并等结果，只认 `handle` 的事务号。发送前先订阅，所以很快收下的也能看到；广播跟不上（`Lagged`）
    /// 时重新订阅并按事务号查当下的队列：还在就继续等，已经出队就当服务器收下了（取消会另发一次更新，漏掉这一条
    /// 最多多转发一次）。队列停掉时同样先查队列，回显还在才算没发出去。
    pub async fn wait_outcome(
        &self,
        updates: Receiver<RoomSendQueueUpdate>,
        handle: SendHandle,
    ) -> SendOutcome {
        let transaction_id = handle.transaction_id().to_owned();
        let echoes = match self.send_updates().await {
            Ok((echoes, _)) => echoes,
            Err(error) => return SendOutcome::Failed(SendFailure::Unreadable(error.to_string())),
        };
        if !has_echo(&echoes, &transaction_id) {
            return SendOutcome::Sent;
        }
        if let Some(error) = wedged(&echoes, &transaction_id) {
            return SendOutcome::Failed(SendFailure::Server(error));
        }
        self.receive_outcome(updates, &transaction_id).await
    }

    async fn receive_outcome(
        &self,
        mut updates: Receiver<RoomSendQueueUpdate>,
        transaction_id: &TransactionId,
    ) -> SendOutcome {
        loop {
            match updates.recv().await {
                Ok(RoomSendQueueUpdate::SentEvent {
                    transaction_id: sent,
                    ..
                }) if sent == transaction_id => return SendOutcome::Sent,
                Ok(RoomSendQueueUpdate::SendError {
                    transaction_id: failed,
                    error,
                    is_recoverable: false,
                }) if failed == transaction_id => {
                    return SendOutcome::Failed(SendFailure::Server(error.to_string()));
                }
                // 只有直播路径能拿到：初次检查和 `Closed` 都是按回显在不在判（不在就当已发出、在就按队列当下状态报），
                // 不看取消。
                Ok(RoomSendQueueUpdate::CancelledLocalEvent {
                    transaction_id: cancelled,
                }) if cancelled == transaction_id => {
                    return SendOutcome::Failed(SendFailure::Cancelled);
                }
                Ok(_) => {}
                Err(RecvError::Lagged(_)) => match self.send_updates().await {
                    Ok((echoes, next)) => {
                        updates = next;
                        if !has_echo(&echoes, transaction_id) {
                            return SendOutcome::Sent;
                        }
                        if let Some(error) = wedged(&echoes, transaction_id) {
                            return SendOutcome::Failed(SendFailure::Server(error));
                        }
                    }
                    Err(error) => {
                        return SendOutcome::Failed(SendFailure::Unreadable(error.to_string()));
                    }
                },
                Err(RecvError::Closed) => {
                    return match self.send_updates().await {
                        Ok((echoes, _)) if !has_echo(&echoes, transaction_id) => SendOutcome::Sent,
                        Ok((echoes, _)) => match wedged(&echoes, transaction_id) {
                            Some(error) => SendOutcome::Failed(SendFailure::Server(error)),
                            None => SendOutcome::Failed(SendFailure::QueueStopped),
                        },
                        Err(error) => {
                            SendOutcome::Failed(SendFailure::Unreadable(error.to_string()))
                        }
                    };
                }
            }
        }
    }
}

fn has_echo(echoes: &[LocalEcho], transaction_id: &TransactionId) -> bool {
    echoes
        .iter()
        .any(|echo| echo.transaction_id.as_str() == transaction_id.as_str())
}

/// 本地回显里记下的不可恢复错误。
fn wedged(echoes: &[LocalEcho], transaction_id: &TransactionId) -> Option<String> {
    echoes
        .iter()
        .find(|echo| echo.transaction_id.as_str() == transaction_id)
        .and_then(|echo| match &echo.content {
            LocalEchoContent::Event {
                send_error: Some(error),
                ..
            } => Some(error.to_string()),
            _ => None,
        })
}
