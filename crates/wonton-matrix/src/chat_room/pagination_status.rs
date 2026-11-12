//! 往回翻页的状态流。

use std::{
    pin::Pin,
    task::{Context, Poll},
};

use futures_util::{Stream, StreamExt};
use tokio::{sync::mpsc, task::JoinHandle};

use super::{BackPaginationStatus, ChatRoom, back_pagination};

impl ChatRoom {
    /// 向前翻页的状态流：第一项是当前状态。
    ///
    /// SDK 的状态流（`Timeline::live_back_pagination_status`）不只是取值：状态变成“到了开头”时，它在被轮询的时候去拿
    /// 时间线的状态锁，补上“房间开头”那一项。读的一方要是在两次轮询之间自己去等这把锁（例如问时间线里有没有某条事件），
    /// SDK 的流就停在排着队的写锁上没人推；tokio 的读写锁是公平的，后来的读锁排在它后面，整条时间线卡死，翻回来的条目
    /// 再也出不来。所以由单独的任务一直推着 SDK 的流，读的一方拿到的是转发出来的状态；丢掉这个流时任务跟着停。
    pub async fn back_pagination_status(
        &self,
    ) -> impl Stream<Item = BackPaginationStatus> + Unpin + use<> {
        let Some((current, updates)) = self.timeline().await.live_back_pagination_status().await
        else {
            // 不是实时时间线（旁聊的话题时间线）没有这个状态：给一次 Idle，之后不再变。流不能结束，
            // 调用方把流结束当成时间线关了。
            return Statuses {
                first: Some(BackPaginationStatus::Idle),
                updates: None,
                driver: None,
            };
        };
        let (sender, receiver) = mpsc::unbounded_channel();
        let driver = tokio::spawn(async move {
            let mut updates = std::pin::pin!(updates);
            while let Some(status) = updates.next().await {
                if sender.send(back_pagination(status)).is_err() {
                    return;
                }
            }
        });
        Statuses {
            first: Some(back_pagination(current)),
            updates: Some(receiver),
            driver: Some(driver),
        }
    }
}

/// 转发出来的翻页状态。
struct Statuses {
    first: Option<BackPaginationStatus>,
    updates: Option<mpsc::UnboundedReceiver<BackPaginationStatus>>,
    driver: Option<JoinHandle<()>>,
}

impl Stream for Statuses {
    type Item = BackPaginationStatus;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if let Some(first) = self.first.take() {
            return Poll::Ready(Some(first));
        }
        match &mut self.updates {
            Some(updates) => updates.poll_recv(cx),
            None => Poll::Pending,
        }
    }
}

impl Drop for Statuses {
    fn drop(&mut self) {
        if let Some(driver) = &self.driver {
            driver.abort();
        }
    }
}
