//! 原生相册页的导出（ADR-0113 第 9 节）：订阅、当前值和字节的本机回环地址。用户操作走 `dispatch(Action::Photos)`。

use std::sync::Arc;

use wonton_protocol::media::MediaError;
use wonton_protocol::media::library_view::{PhotoRendition, PhotosViewModel};

use super::ClientCore;
use super::media_sources::run;
use crate::photos::PhotosSubscription;

#[uniffi::export]
impl ClientCore {
    /// 当前的相册页，用于首帧渲染。
    pub fn photos(&self) -> PhotosViewModel {
        self.views().photos.borrow().clone()
    }

    /// 订阅相册页，语义同 [`subscribe_session_list`](Self::subscribe_session_list)。
    pub fn subscribe_photos(&self) -> Arc<PhotosSubscription> {
        Arc::new(PhotosSubscription::new(self.views().photos.subscribe()))
    }

    /// 一条的这一档字节的本机回环地址，交给系统的图片加载器或播放器，`Range` 原样过。地址只在这次启动里有效，
    /// 端口可能变：拿它当缓存键的，换成条目 id 加档位。
    pub async fn photo_url(
        &self,
        id: String,
        rendition: PhotoRendition,
    ) -> Result<String, MediaError> {
        #[cfg(feature = "ui-fake")]
        if let Some(url) = self.ui_fake_photo_url(&id, rendition) {
            return url;
        }
        let scope = self.scope();
        let photos = scope.photos.clone();
        run(&scope, async move { photos.url(&id, rendition).await }).await
    }

    /// 一个人物（[`PhotoPerson`](wonton_protocol::media::library_view::PhotoPerson)）的头像的本机回环地址。
    pub async fn photo_person_url(&self, id: String) -> Result<String, MediaError> {
        #[cfg(feature = "ui-fake")]
        if let Some(url) = self.ui_fake_person_url(&id) {
            return url;
        }
        let scope = self.scope();
        let photos = scope.photos.clone();
        run(&scope, async move { photos.person_url(&id).await }).await
    }
}
