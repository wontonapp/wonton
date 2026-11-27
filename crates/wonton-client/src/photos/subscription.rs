use tokio::sync::watch;
use wonton_protocol::media::library_view::PhotosViewModel;

use crate::subscription::Updates;

/// 相册页（ADR-0113 第 9 节）的订阅，用法同 [`SessionListSubscription`](crate::SessionListSubscription)。
#[derive(uniffi::Object)]
pub struct PhotosSubscription {
    updates: Updates<PhotosViewModel>,
}

impl PhotosSubscription {
    pub(crate) fn new(receiver: watch::Receiver<PhotosViewModel>) -> Self {
        Self {
            updates: Updates::new(receiver),
        }
    }
}

#[uniffi::export]
impl PhotosSubscription {
    /// 等待下一份 ViewModel，语义同 [`SessionListSubscription::next`](crate::SessionListSubscription::next)。
    pub async fn next(&self) -> Option<PhotosViewModel> {
        self.updates.next(PhotosViewModel::clone).await
    }

    /// 取消订阅，语义同 [`SessionListSubscription::cancel`](crate::SessionListSubscription::cancel)。
    pub fn cancel(&self) {
        self.updates.cancel();
    }
}
