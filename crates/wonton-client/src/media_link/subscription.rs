//! 媒体的两个订阅。壳循环调 `next()`，包装成 Swift 的 `AsyncSequence` 或 Kotlin 的 `Flow`；不再需要时调
//! `cancel()`，之后 `next()` 回空，不再收片。

use tokio::sync::{Mutex, watch};
use wonton_protocol::media::{MediaViewModel, PlaybackViewModel};

/// 等 watch 通道的下一个值，可以取消；第一次 `next()` 立即拿到当前值。
struct Updates<T> {
    receiver: Mutex<watch::Receiver<T>>,
    cancelled: watch::Sender<bool>,
}

impl<T: Clone> Updates<T> {
    fn new(mut receiver: watch::Receiver<T>) -> Self {
        receiver.mark_changed();
        Self {
            receiver: Mutex::new(receiver),
            cancelled: watch::Sender::new(false),
        }
    }

    /// 下一个让 `keep` 认的值；取消了或发送端没了回空。
    async fn next_where<R>(&self, keep: impl Fn(&T) -> Option<R>) -> Option<R> {
        let mut cancelled = self.cancelled.subscribe();
        let mut receiver = self.receiver.lock().await;
        loop {
            if *cancelled.borrow_and_update() {
                return None;
            }
            tokio::select! {
                biased;
                _ = cancelled.wait_for(|cancelled| *cancelled) => return None,
                changed = receiver.changed() => changed.ok()?,
            }
            if let Some(value) = keep(&receiver.borrow_and_update()) {
                return Some(value);
            }
        }
    }

    fn cancel(&self) {
        self.cancelled.send_replace(true);
    }
}

/// 任务列表和配额。
#[derive(uniffi::Object)]
pub struct MediaSubscription {
    updates: Updates<MediaViewModel>,
}

impl MediaSubscription {
    pub(crate) fn new(receiver: watch::Receiver<MediaViewModel>) -> Self {
        Self {
            updates: Updates::new(receiver),
        }
    }
}

#[uniffi::export]
impl MediaSubscription {
    pub async fn next(&self) -> Option<MediaViewModel> {
        self.updates.next_where(|view| Some(view.clone())).await
    }

    pub fn cancel(&self) {
        self.updates.cancel();
    }
}

/// 这个人的播放：还没有会话时不出值，有了以后每次变化出一次（停了的会话也出，壳显示停在哪）。
#[derive(uniffi::Object)]
pub struct PlaybackSubscription {
    updates: Updates<Option<PlaybackViewModel>>,
}

impl PlaybackSubscription {
    pub(crate) fn new(receiver: watch::Receiver<Option<PlaybackViewModel>>) -> Self {
        Self {
            updates: Updates::new(receiver),
        }
    }
}

#[uniffi::export]
impl PlaybackSubscription {
    pub async fn next(&self) -> Option<PlaybackViewModel> {
        self.updates.next_where(Option::clone).await
    }

    pub fn cancel(&self) {
        self.updates.cancel();
    }
}
