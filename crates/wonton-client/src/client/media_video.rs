//! 影视的导出（ADR-0113 第 10 节）：首页、海报墙、详情、标看过和收藏、起播、离线下载。图片地址、能不能放、
//! 从哪接着放都由客户端核算好，壳只画和转发动作。

use wonton_protocol::media::catalog::CatalogAction;
use wonton_protocol::media::{
    MediaError, MediaItemRef, MediaTask, TaskCreate, TaskKind, TaskOptions, TaskTarget,
};

#[cfg(feature = "ui-fake")]
use std::sync::atomic::Ordering;

use super::ClientCore;
use super::media_sources::run;
use crate::media_link::{
    OfflineState, VideoActionResult, VideoDetail, VideoHome, VideoPlayback, VideoWall,
    VideoWallQuery, session_id,
};

#[uniffi::export]
impl ClientCore {
    /// 影视首页：库、继续看、下一集、最近添加。`source` 空着由核心按默认选媒体源。
    pub async fn video_home(&self, source: Option<String>) -> Result<VideoHome, MediaError> {
        #[cfg(feature = "ui-fake")]
        if self.ui_fake.load(Ordering::SeqCst) {
            return Ok(super::ui_fake_video::home());
        }
        let scope = self.scope();
        let media = scope.media.clone();
        run(&scope, async move { media.video_home(source).await }).await
    }

    /// 海报墙的一页。
    pub async fn video_wall(&self, query: VideoWallQuery) -> Result<VideoWall, MediaError> {
        #[cfg(feature = "ui-fake")]
        if self.ui_fake.load(Ordering::SeqCst) {
            return Ok(super::ui_fake_video::wall(&query));
        }
        let scope = self.scope();
        let media = scope.media.clone();
        run(&scope, async move { media.video_wall(query).await }).await
    }

    /// 详情：剧带季和该看的那一集，季带集，能放的带音轨、字幕和这台设备上的离线副本。
    pub async fn video_detail(
        &self,
        id: String,
        source: Option<String>,
    ) -> Result<VideoDetail, MediaError> {
        #[cfg(feature = "ui-fake")]
        if self.ui_fake.load(Ordering::SeqCst) {
            return Ok(super::ui_fake_video::detail(&id));
        }
        let scope = self.scope();
        let media = scope.media.clone();
        run(&scope, async move { media.video_detail(id, source).await }).await
    }

    /// 某个库（空着是全部）有哪些类型，给海报墙筛。
    pub async fn video_genres(
        &self,
        library: Option<String>,
        source: Option<String>,
    ) -> Result<Vec<String>, MediaError> {
        #[cfg(feature = "ui-fake")]
        if self.ui_fake.load(Ordering::SeqCst) {
            return Ok(super::ui_fake_video::genres());
        }
        let scope = self.scope();
        let media = scope.media.clone();
        run(
            &scope,
            async move { media.video_genres(library, source).await },
        )
        .await
    }

    /// 标看过或没看过；剧、季会连同下面的集一起标。
    pub async fn video_set_played(
        &self,
        ids: Vec<String>,
        played: bool,
        source: Option<String>,
    ) -> Result<VideoActionResult, MediaError> {
        #[cfg(feature = "ui-fake")]
        if self.ui_fake.load(Ordering::SeqCst) {
            return Ok(fake_changed(&ids));
        }
        let scope = self.scope();
        let media = scope.media.clone();
        let action = CatalogAction::SetPlayed { ids, played };
        run(&scope, async move { media.video_act(action, source).await }).await
    }

    pub async fn video_set_favorite(
        &self,
        ids: Vec<String>,
        favorite: bool,
        source: Option<String>,
    ) -> Result<VideoActionResult, MediaError> {
        #[cfg(feature = "ui-fake")]
        if self.ui_fake.load(Ordering::SeqCst) {
            return Ok(fake_changed(&ids));
        }
        let scope = self.scope();
        let media = scope.media.clone();
        let action = CatalogAction::Favorite { ids, favorite };
        run(&scope, async move { media.video_act(action, source).await }).await
    }

    /// 起播一条（卡上的 `item`）。`position_ms` 是从哪放：接着看传卡上的 `position_ms`，从头传 0；
    /// 音轨、字幕空着用提供方的默认。回来的 `url` 播放器直接打开。
    pub async fn video_play(
        &self,
        item: MediaItemRef,
        position_ms: u64,
        audio: Option<u16>,
        subtitle: Option<u16>,
    ) -> Result<VideoPlayback, MediaError> {
        #[cfg(feature = "ui-fake")]
        if self.ui_fake.load(Ordering::SeqCst) {
            return Err(super::ui_fake_video::no_stream().into());
        }
        let scope = self.scope();
        let media = scope.media.clone();
        run(&scope, async move {
            media.video_play(item, position_ms, audio, subtitle).await
        })
        .await
    }

    /// 播放器的位置：播放中每秒调一次就行，核心按节奏报；暂停、拖动后传 `force`。
    pub async fn video_progress(
        &self,
        session: String,
        position_ms: u64,
        force: bool,
    ) -> Result<(), MediaError> {
        #[cfg(feature = "ui-fake")]
        if self.ui_fake.load(Ordering::SeqCst) {
            return Ok(());
        }
        let session = session_id(&session)?;
        let scope = self.scope();
        let media = scope.media.clone();
        run(&scope, async move {
            media.video_progress(session, position_ms, force).await
        })
        .await
    }

    /// 播放器关了：停在哪，是不是放完了。
    pub async fn video_stop(
        &self,
        session: String,
        position_ms: u64,
        finished: bool,
    ) -> Result<(), MediaError> {
        #[cfg(feature = "ui-fake")]
        if self.ui_fake.load(Ordering::SeqCst) {
            return Ok(());
        }
        let session = session_id(&session)?;
        let scope = self.scope();
        let media = scope.media.clone();
        run(&scope, async move {
            media.video_stop(session, position_ms, finished).await
        })
        .await
    }

    /// 把一条下到这台设备上（核心建任务，这台设备接着就下）；进度看 `subscribe_media`。
    pub async fn video_download(&self, item: MediaItemRef) -> Result<MediaTask, MediaError> {
        #[cfg(feature = "ui-fake")]
        if self.ui_fake.load(Ordering::SeqCst) {
            return Err(super::ui_fake_video::no_stream().into());
        }
        let scope = self.scope();
        let media = scope.media.clone();
        let create = TaskCreate {
            kind: TaskKind::Download,
            person: String::new(),
            device: String::new(),
            target: TaskTarget::Download(item),
            options: TaskOptions::default(),
        };
        run(&scope, async move { media.create_task(create).await }).await
    }

    /// 这一条在这台设备上的离线副本；没下过为空。
    pub fn video_offline(&self, item_id: String) -> Option<OfflineState> {
        #[cfg(feature = "ui-fake")]
        if self.ui_fake.load(Ordering::SeqCst) {
            return super::ui_fake_video::offline(&item_id);
        }
        self.scope().media.offline(&item_id)
    }

    /// 删掉这一条在这台设备上的离线副本。
    pub fn video_remove_offline(&self, item_id: String) -> Result<(), MediaError> {
        #[cfg(feature = "ui-fake")]
        if self.ui_fake.load(Ordering::SeqCst) {
            super::ui_fake_video::remove_offline(&item_id);
            return Ok(());
        }
        Ok(self.scope().media.remove_offline(&item_id)?)
    }

    /// 离线副本是 `Failed` 时再下一次，接着上次下到的地方。上一次重试还没回来时再调什么也不做。
    pub async fn video_retry_download(&self, item_id: String) -> Result<(), MediaError> {
        #[cfg(feature = "ui-fake")]
        if self.ui_fake.load(Ordering::SeqCst) {
            return Err(super::ui_fake_video::no_stream().into());
        }
        let scope = self.scope();
        let media = scope.media.clone();
        run(&scope, async move { media.retry_download(&item_id).await }).await
    }

    /// 离线副本是 `Paused` 时接着下。
    pub async fn video_resume_download(&self, item_id: String) -> Result<(), MediaError> {
        #[cfg(feature = "ui-fake")]
        if self.ui_fake.load(Ordering::SeqCst) {
            return Err(super::ui_fake_video::no_stream().into());
        }
        let scope = self.scope();
        let media = scope.media.clone();
        run(&scope, async move { media.resume_download(&item_id).await }).await
    }

    /// 当前服务器和账号的标识。壳的图片缓存按它分开：换了服务器或账号，同一条路径是另一张图。
    pub fn video_cache_scope(&self) -> String {
        self.scope().slot.clone()
    }
}

/// 假数据入口里标看过、收藏都算成了，不落到哪里。
#[cfg(feature = "ui-fake")]
fn fake_changed(ids: &[String]) -> VideoActionResult {
    VideoActionResult {
        changed: u32::try_from(ids.len()).unwrap_or(u32::MAX),
        failed: Vec::new(),
    }
}
