use std::sync::Arc;

use tokio::sync::{Mutex, watch};
use wonton_protocol::music::{
    MusicAction, MusicLyricsState, MusicPage, MusicPageAction, MusicPageView,
};

use wonton_protocol::media::{MediaError, MediaFailureCode};

use super::ClientCore;
use crate::media_link::{MediaLink, failure};
use crate::music_library::{MusicLink, Page, playable};

/// 原生音乐页的一页（ADR-0113 第 2 节）。壳循环调 [`next`](Self::next) 拿新的视图，页内动作发 [`act`](Self::act)，
/// 页面关掉时调 [`cancel`](Self::cancel)。
#[derive(uniffi::Object)]
pub struct MusicPageSubscription {
    link: Arc<MusicLink>,
    page: Arc<Page>,
    media: Arc<MediaLink>,
    receiver: Mutex<watch::Receiver<MusicPageView>>,
    cancelled: watch::Sender<bool>,
}

#[uniffi::export]
impl MusicPageSubscription {
    pub fn view(&self) -> MusicPageView {
        self.page.view()
    }

    pub async fn next(&self) -> Option<MusicPageView> {
        let mut cancelled = self.cancelled.subscribe();
        let mut receiver = self.receiver.lock().await;
        tokio::select! {
            biased;
            _ = cancelled.wait_for(|cancelled| *cancelled) => None,
            changed = receiver.changed() => {
                changed.ok()?;
                Some(receiver.borrow_and_update().clone())
            }
        }
    }

    pub fn act(&self, action: MusicPageAction) {
        self.link.act(&self.page, action);
    }

    /// 放这一页上的一首歌：这一页的歌按页面顺序就是要放的那一列。同一首在歌单里出现两次时从第一次起。
    /// 出声、进度、换曲之后都看播放视图。
    pub async fn play(&self, song_id: String) -> Result<(), MediaError> {
        let items = playable(&self.page.view(), self.page.plugin_id());
        let index = items
            .iter()
            .position(|item| item.id == song_id)
            .ok_or_else(|| {
                failure(
                    MediaFailureCode::MediaGone,
                    "song is no longer on this page",
                )
            })?;
        Ok(self.media.play_items(items, index).await?)
    }

    pub fn cancel(&self) {
        if !self.cancelled.send_replace(true) {
            self.link.close(&self.page);
        }
    }
}

impl Drop for MusicPageSubscription {
    fn drop(&mut self) {
        self.cancel();
    }
}

#[uniffi::export]
impl ClientCore {
    /// 打开音乐插件 `plugin_id` 的一页并开始取；第一次 `next()` 立即回当前视图。
    pub fn subscribe_music_page(
        &self,
        plugin_id: String,
        page: MusicPage,
    ) -> Arc<MusicPageSubscription> {
        let link = self.music_link();
        let page = link.open(plugin_id, page);
        let mut receiver = page.subscribe();
        receiver.mark_changed();
        Arc::new(MusicPageSubscription {
            link,
            page,
            media: self.scope().media.clone(),
            receiver: Mutex::new(receiver),
            cancelled: watch::Sender::new(false),
        })
    }

    /// 改到曲库的动作（加星）：开着的页立刻改；曲库拒了改回来，回原因，成功回空。
    pub async fn music_act(
        &self,
        plugin_id: String,
        action: MusicAction,
    ) -> Option<wonton_protocol::music::MusicProblem> {
        let link = self.music_link();
        self.scope()
            .run(async move { Ok(link.star(&plugin_id, action).await) })
            .await
            .unwrap_or_else(|error| {
                Some(wonton_protocol::music::MusicProblem {
                    failure: wonton_protocol::music::MusicFailure::Transport,
                    details: Some(error.to_string()),
                })
            })
    }

    /// 一首歌的歌词。
    pub async fn music_lyrics(&self, plugin_id: String, song_id: String) -> MusicLyricsState {
        let link = self.music_link();
        let result = self
            .scope()
            .run(async move { Ok(link.lyrics(&plugin_id, &song_id).await) })
            .await;
        match result {
            Ok(Ok(Some(lyrics))) => MusicLyricsState::Found { lyrics },
            Ok(Ok(None)) => MusicLyricsState::Missing,
            Ok(Err(problem)) => MusicLyricsState::Failed { problem },
            Err(error) => MusicLyricsState::Failed {
                problem: wonton_protocol::music::MusicProblem {
                    failure: wonton_protocol::music::MusicFailure::Transport,
                    details: Some(error.to_string()),
                },
            },
        }
    }

    /// 封面图片字节，`size` 是要显示的边长像素；取不到时为空，壳画占位。
    pub async fn music_cover(
        &self,
        plugin_id: String,
        cover: String,
        size: u32,
    ) -> Option<Vec<u8>> {
        let link = self.music_link();
        self.scope()
            .run(async move { Ok(link.cover(&plugin_id, &cover, size).await) })
            .await
            .ok()
            .flatten()
    }
}

impl ClientCore {
    /// 音乐页用的浏览层：假数据入口下是本机的假曲库。
    fn music_link(&self) -> Arc<MusicLink> {
        #[cfg(feature = "ui-fake")]
        if self.ui_fake.load(std::sync::atomic::Ordering::SeqCst) {
            return super::ui_fake::music_link(&self.root);
        }
        self.scope().music.clone()
    }
}
