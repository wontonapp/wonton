//! 从原生页起播，和播放器要的两样地址（ADR-0113 第 4 节）：流走本机回环，封面经核心找同一家提供方。

use std::sync::Arc;
use std::time::Duration;

use reqwest::Method;
use wonton_protocol::media::api::{
    MEDIA_API_PREFIX, item_artwork_path, queue_path, session_stream_path,
};
use wonton_protocol::media::provider::ArtworkQuery;
use wonton_protocol::media::{
    DeviceProfile, MediaFailure, MediaFailureCode, MediaItemRef, PlaybackRequest,
    PlaybackSessionId, Queue, QueueOp,
};

use super::MediaLink;
use super::api::{failure, send};
use super::queue;

/// 取一张封面最多等这么久：封面要经核心再到提供方，比控制请求慢，但不该挂着不回。
const ARTWORK_TIMEOUT: Duration = Duration::from_secs(30);

impl MediaLink {
    /// 从 `items` 的第 `index` 项起放：先把这个人的队列整条换成 `items`、指到这一项，再起播它。之后放完、
    /// 下一首、上一首都按这条队列走（换曲由核心按队列的指针做）。
    pub(crate) async fn play_items(
        self: &Arc<Self>,
        items: Vec<MediaItemRef>,
        index: usize,
    ) -> Result<(), MediaFailure> {
        let item = items.get(index).cloned().ok_or_else(|| {
            failure(
                MediaFailureCode::MediaGone,
                "item index is past the end of the list",
            )
        })?;
        let link = self.clone();
        self.runtime
            .spawn(async move {
                link.replace_queue(items, index).await?;
                link.start_playback(request(item)).await.map(drop)
            })
            .await
            .map_err(|error| failure(MediaFailureCode::UpstreamError, error.to_string()))?
    }

    /// 队列整条换掉。本机还没有队列（连上时没取到）就先取一次，拿它的版本去改；别处先改了由
    /// [`queue::change`] 换上新版本原样再发一次。
    async fn replace_queue(
        &self,
        entries: Vec<MediaItemRef>,
        cursor: usize,
    ) -> Result<(), MediaFailure> {
        let api = self.api()?;
        let known = self.lock().queue().cloned();
        let before = match known {
            Some(queue) => queue,
            None => api.get::<Queue>(&queue_path()).await?,
        };
        let op = QueueOp::Replace {
            entries,
            cursor: Some(cursor),
        };
        queue::change(&api, Some(&before), before.revision, op, |queue| {
            self.update(|local| local.put_queue(queue));
        })
        .await
    }

    /// 播放器连的地址：本机回环加这条会话的流。地址只认会话，换了条目（下一首）还是这个地址、内容换了，
    /// 壳看播放视图里的条目变了就重新打开。
    pub(crate) async fn stream_url(
        &self,
        session: PlaybackSessionId,
    ) -> Result<String, MediaFailure> {
        let prefix = self.loopback().await?;
        let path = session_stream_path(session);
        let rest = path
            .strip_prefix(MEDIA_API_PREFIX)
            .expect("media api paths start with the media prefix");
        Ok(format!("{prefix}{rest}"))
    }

    /// 条目的封面，`width` 是要显示的像素宽；提供方没有、取不到都回失败，壳画占位。
    pub(crate) async fn artwork(
        &self,
        item: &MediaItemRef,
        width: u32,
    ) -> Result<Vec<u8>, MediaFailure> {
        let query = ArtworkQuery {
            artwork: None,
            width: Some(width),
        };
        let path = item_artwork_path(item, &query);
        let request = self
            .api()?
            .request(Method::GET, &path)
            .timeout(ARTWORK_TIMEOUT);
        let response = send(request).await?;
        let bytes = response
            .bytes()
            .await
            .map_err(|e| failure(MediaFailureCode::TransportUnavailable, e.to_string()))?;
        Ok(bytes.to_vec())
    }
}

/// 起播一项：设备、人留空由核心按这台设备填，设备档用壳报过的。
fn request(item: MediaItemRef) -> PlaybackRequest {
    PlaybackRequest {
        position_ms: 0,
        audio: None,
        subtitle: None,
        item,
        device: String::new(),
        person: String::new(),
        profile: DeviceProfile {
            direct_play: Vec::new(),
            max_bitrate: 0,
            hdr: false,
            container: Vec::new(),
        },
    }
}
