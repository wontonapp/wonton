//! 媒体的导出：两个订阅、播放器状态、起播、建任务、浏览条目（ADR-0113）。播放控制、停止和队列走
//! `dispatch(Action::Media)`。

use std::future::Future;
use std::sync::Arc;

use wonton_protocol::media::provider::{ItemsPage, ItemsQuery};
use wonton_protocol::media::{
    DeviceProfile, MediaError, MediaFailure, MediaFailureCode, MediaTask, PlaybackRequest,
    PlaybackViewModel, TaskCreate,
};

use super::ClientCore;
use crate::media_link::{
    MediaSubscription, PlaybackSubscription, PlayerState, PlayerSupport, failure,
};
use crate::scope::Scope;

#[uniffi::export]
impl ClientCore {
    /// 任务列表和配额，语义同 [`subscribe_transfers`](Self::subscribe_transfers)。
    pub fn subscribe_media(&self) -> Arc<MediaSubscription> {
        Arc::new(MediaSubscription::new(self.views().media.media.subscribe()))
    }

    /// 这个人当前的播放会话；还没有会话时不出值。
    pub fn subscribe_playback(&self) -> Arc<PlaybackSubscription> {
        Arc::new(PlaybackSubscription::new(
            self.views().media.playback.subscribe(),
        ))
    }

    pub fn player_state(&self) -> PlayerState {
        self.scope().media.player_state()
    }

    /// 壳报这台设备的播放器能直放什么；之后起播请求里的设备档空着就用它。缺了格式或容器回
    /// `unsupported_by_device`，不记。
    pub fn report_player_support(
        &self,
        support: PlayerSupport,
    ) -> Result<DeviceProfile, MediaError> {
        Ok(self.scope().media.report_player_support(support)?)
    }

    /// 本机回环前缀（带随机令牌）。系统播放器和后台传输只连这里，后面接核心媒体接口
    /// `/_wonton/api/media` 之后的那段。
    pub async fn media_loopback(&self) -> Result<String, MediaError> {
        let scope = self.scope();
        let media = scope.media.clone();
        run(&scope, async move { media.loopback().await }).await
    }

    /// 起播。`person`、`device` 留空，核心按这台设备填；设备档空着用壳报过的。
    pub async fn start_playback(
        &self,
        request: PlaybackRequest,
    ) -> Result<PlaybackViewModel, MediaError> {
        let scope = self.scope();
        let media = scope.media.clone();
        run(&scope, async move { media.start_playback(request).await }).await
    }

    /// 建上传或下载任务。`person`、`device` 留空，核心按这台设备填。
    pub async fn create_media_task(&self, create: TaskCreate) -> Result<MediaTask, MediaError> {
        let scope = self.scope();
        let media = scope.media.clone();
        run(&scope, async move { media.create_task(create).await }).await
    }

    /// 按种类翻提供方的条目（`kind` 必填）。
    pub async fn media_items(&self, query: ItemsQuery) -> Result<ItemsPage, MediaError> {
        let scope = self.scope();
        let media = scope.media.clone();
        run(&scope, async move { media.items(query).await }).await
    }
}

/// 在作用域的运行时里执行，保留失败的类别。
async fn run<T: Send + 'static>(
    scope: &Scope,
    task: impl Future<Output = Result<T, MediaFailure>> + Send + 'static,
) -> Result<T, MediaError> {
    let switched = || {
        failure(
            MediaFailureCode::TransportUnavailable,
            "switched to another server",
        )
    };
    let handle = scope.handle().ok_or_else(|| MediaError::from(switched()))?;
    match handle.spawn(task).await {
        Ok(result) => result.map_err(MediaError::from),
        Err(error) if error.is_cancelled() => Err(switched().into()),
        Err(error) => Err(failure(MediaFailureCode::UpstreamError, error.to_string()).into()),
    }
}
