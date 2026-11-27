//! 从原生页起播，和播放器要的两样地址（ADR-0113 第 4 节）：流走本机回环，封面经核心找同一家提供方。

use std::sync::Arc;
use std::time::Duration;

use reqwest::Method;
use wonton_protocol::media::api::{MEDIA_API_PREFIX, item_artwork_path, session_stream_path};
use wonton_protocol::media::provider::ArtworkQuery;
use wonton_protocol::media::{
    DeviceProfile, MediaFailure, MediaFailureCode, MediaItemRef, PlaybackRequest, PlaybackSessionId,
};

use super::MediaLink;
use super::api::{failure, send};

/// 取一张封面最多等这么久：封面要经核心再到提供方，比控制请求慢，但不该挂着不回。
const ARTWORK_TIMEOUT: Duration = Duration::from_secs(30);

impl MediaLink {
    /// 从 `items` 的第 `index` 项起放。队列整条替换（`QueueOp::Replace`）接上之前只起播这一项。
    pub(crate) async fn play_items(
        self: &Arc<Self>,
        items: Vec<MediaItemRef>,
        index: usize,
    ) -> Result<(), MediaFailure> {
        let item = items.into_iter().nth(index).ok_or_else(|| {
            failure(
                MediaFailureCode::MediaGone,
                "item index is past the end of the list",
            )
        })?;
        let request = PlaybackRequest {
            position_ms: 0,
            audio: None,
            subtitle: None,
            item,
            // 留空由核心按这台设备填，设备档用壳报过的。
            device: String::new(),
            person: String::new(),
            profile: DeviceProfile {
                direct_play: Vec::new(),
                max_bitrate: 0,
                hdr: false,
                container: Vec::new(),
            },
        };
        let link = self.clone();
        self.runtime
            .spawn(async move { link.start_playback(request).await.map(drop) })
            .await
            .map_err(|error| failure(MediaFailureCode::UpstreamError, error.to_string()))?
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
