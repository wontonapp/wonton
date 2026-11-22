//! 成员的显示名和房间的正在输入状态（ADR-0109 §8）。

use futures_util::{Stream, stream};
use matrix_sdk::ruma::{OwnedUserId, UserId};
use tokio::sync::broadcast::error::RecvError;

use super::ChatRoom;

impl ChatRoom {
    /// 成员的显示名：先用成员事件里的名字，没有时是账号名（房间内唯一的兜底，[`MemberNames::name`]）。
    /// 不认识的人也给得出，只是回退到账号名。
    pub fn display_name(&self, user: &UserId) -> String {
        self.names.borrow().name(user)
    }

    /// 房间里正在输入的人；每次变化给一份完整的名单，本账号已经滤掉。流结束时房间里不会再有更新。
    pub fn typing_notifications(&self) -> impl Stream<Item = Vec<OwnedUserId>> + Send + use<> {
        let (guard, receiver) = self.room.subscribe_to_typing_notifications();
        stream::unfold((receiver, guard), |(mut receiver, guard)| async move {
            loop {
                match receiver.recv().await {
                    Ok(users) => return Some((users, (receiver, guard))),
                    // 掉过一条就只拿到最新的快照，够用：名字是完整名单，不是增量。
                    Err(RecvError::Lagged(_)) => continue,
                    Err(RecvError::Closed) => return None,
                }
            }
        })
    }
}
