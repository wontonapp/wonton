//! 影视的导出（ADR-0113 第 10 节）：首页、海报墙、详情、标看过和收藏、起播、离线下载。图片地址、能不能放、
//! 从哪接着放都由客户端核算好，壳只画和转发动作。

use wonton_protocol::media::catalog::CatalogAction;
use wonton_protocol::media::{
    DeviceProfile, MediaError, MediaItemRef, MediaTask, PlaybackRequest, PlaybackViewModel,
    TaskCreate, TaskKind, TaskOptions, TaskTarget,
};

use super::ClientCore;
use super::media_sources::run;
use crate::media_link::{
    OfflineState, VideoActionResult, VideoDetail, VideoHome, VideoWall, VideoWallQuery,
};

#[uniffi::export]
impl ClientCore {
    /// 影视首页：库、继续看、下一集、最近添加。`source` 空着由核心按默认选媒体源。
    pub async fn video_home(&self, source: Option<String>) -> Result<VideoHome, MediaError> {
        let scope = self.scope();
        let media = scope.media.clone();
        run(&scope, async move { media.video_home(source).await }).await
    }

    /// 海报墙的一页。
    pub async fn video_wall(&self, query: VideoWallQuery) -> Result<VideoWall, MediaError> {
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
        let scope = self.scope();
        let media = scope.media.clone();
        let action = CatalogAction::Favorite { ids, favorite };
        run(&scope, async move { media.video_act(action, source).await }).await
    }

    /// 起播一条（卡上的 `item`）。`position_ms` 是从哪放：接着看传卡上的 `position_ms`，从头传 0；
    /// 音轨、字幕空着用提供方的默认。
    pub async fn video_play(
        &self,
        item: MediaItemRef,
        position_ms: u64,
        audio: Option<u16>,
        subtitle: Option<u16>,
    ) -> Result<PlaybackViewModel, MediaError> {
        let scope = self.scope();
        let media = scope.media.clone();
        let request = PlaybackRequest {
            position_ms,
            audio,
            subtitle,
            item,
            device: String::new(),
            person: String::new(),
            profile: DeviceProfile {
                direct_play: Vec::new(),
                max_bitrate: 0,
                hdr: false,
                container: Vec::new(),
            },
        };
        run(&scope, async move { media.start_playback(request).await }).await
    }

    /// 把一条下到这台设备上（核心建任务，这台设备接着就下）；进度看 `subscribe_media`。
    pub async fn video_download(&self, item: MediaItemRef) -> Result<MediaTask, MediaError> {
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
        self.scope().media.offline(&item_id)
    }

    /// 删掉这一条在这台设备上的离线副本。
    pub fn video_remove_offline(&self, item_id: String) -> Result<(), MediaError> {
        Ok(self.scope().media.remove_offline(&item_id)?)
    }
}
