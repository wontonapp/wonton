//! 影视的起播和进度（ADR-0113 第 4 节）：起播请求由客户端核拼，回来的会话翻成播放器直接打开的地址；
//! 播放器的位置按节奏报给核心，核心再转给提供方（“继续看”那一排靠它）。

use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use wonton_protocol::media::api::{MEDIA_API_PREFIX, session_progress_path, session_stop_path};
use wonton_protocol::media::provider::{PlaybackProgress, PlaybackStopped};
use wonton_protocol::media::{
    DeviceProfile, MediaFailure, MediaFailureCode, MediaItemRef, MediaTrack, PlaybackRequest,
    PlaybackSessionId, PlaybackViewModel, ProviderStream, StopReason,
};

use super::MediaLink;
use super::api::failure;
use crate::l10n::text;

/// 播放中最多这么久报一次位置（第 3 节：播放每 5 秒，暂停、拖动、停止时各一次）。
const REPORT_EVERY: Duration = Duration::from_secs(5);

/// 起播后播放器要的东西。
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct VideoPlayback {
    /// 播放会话的号，报进度和停止时原样交回。
    pub session: String,
    /// 本机回环上的流地址，播放器直接打开。
    pub url: String,
    /// 核心那边在转码（HLS）；为假是原片直放。
    pub transcoded: bool,
    /// 从哪开始放。
    pub position_ms: u64,
    pub duration_ms: u64,
    pub seekable: bool,
    pub audio: Vec<MediaTrack>,
    pub subtitles: Vec<MediaTrack>,
}

/// 上一次报位置的会话和时间。
#[derive(Default)]
pub(crate) struct Reports(Mutex<Option<(PlaybackSessionId, Instant)>>);

impl Reports {
    /// 这一次该不该报：换了会话、强制、或者离上次够久了。
    fn due(&self, session: PlaybackSessionId, force: bool) -> bool {
        let mut last = self.0.lock().unwrap_or_else(|p| p.into_inner());
        let due =
            force || last.is_none_or(|(id, at)| id != session || at.elapsed() >= REPORT_EVERY);
        if due {
            *last = Some((session, Instant::now()));
        }
        due
    }
}

/// 壳交回来的会话号。号在 FFI 上是字符串，认不出来是壳传错了。
pub(crate) fn session_id(value: &str) -> Result<PlaybackSessionId, MediaFailure> {
    PlaybackSessionId::parse_str(value).map_err(|_| {
        failure(
            MediaFailureCode::Protocol,
            text("video-session-unreadable", &[("value", value.into())]),
        )
    })
}

/// 会话翻成播放器要的：流地址接在本机回环前缀后面（核心给的是 `/_wonton/api/media/…` 的路径）。
pub(crate) fn playback(
    prefix: &str,
    view: &PlaybackViewModel,
) -> Result<VideoPlayback, MediaFailure> {
    let plan = view
        .plan
        .as_ref()
        .ok_or_else(|| failure(MediaFailureCode::Protocol, text("video-plan-missing", &[])))?;
    let (path, transcoded) = match &plan.stream {
        ProviderStream::Direct(path) => (path, false),
        ProviderStream::Transcoded(path) => (path, true),
        ProviderStream::Other(path) => (path, true),
    };
    let rest = path.strip_prefix(MEDIA_API_PREFIX).ok_or_else(|| {
        failure(
            MediaFailureCode::Protocol,
            text(
                "video-stream-outside-api",
                &[("value", path.as_str().into())],
            ),
        )
    })?;
    Ok(VideoPlayback {
        session: view.session.id.to_string(),
        url: format!("{prefix}{rest}"),
        transcoded,
        position_ms: view.session.position_ms,
        duration_ms: plan.duration_ms,
        seekable: plan.seekable,
        audio: plan.audio.clone(),
        subtitles: plan.subtitles.clone(),
    })
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

impl MediaLink {
    /// 起播一条：`position_ms` 从哪放，音轨、字幕空着用提供方的默认。设备档用壳报过的。
    pub(crate) async fn video_play(
        &self,
        item: MediaItemRef,
        position_ms: u64,
        audio: Option<u16>,
        subtitle: Option<u16>,
    ) -> Result<VideoPlayback, MediaFailure> {
        let prefix = self.loopback().await?;
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
        let view = self.start_playback(request).await?;
        playback(&prefix, &view)
    }

    /// 播放器报位置。播放中按节奏报，`force` 的（暂停、拖动后）马上报；没到点的直接返回。
    pub(crate) async fn video_progress(
        &self,
        session: PlaybackSessionId,
        position_ms: u64,
        force: bool,
    ) -> Result<(), MediaFailure> {
        if !self.reports.due(session, force) {
            return Ok(());
        }
        let report = PlaybackProgress {
            position_ms,
            wall_clock: now_ms(),
        };
        self.api()?
            .post_empty(&session_progress_path(session), &report)
            .await
    }

    /// 播放器关了：报停在哪，放完了的算看完。
    pub(crate) async fn video_stop(
        &self,
        session: PlaybackSessionId,
        position_ms: u64,
        finished: bool,
    ) -> Result<(), MediaFailure> {
        let body = PlaybackStopped {
            position_ms,
            reason: if finished {
                StopReason::Finished
            } else {
                StopReason::User
            },
        };
        self.api()?
            .post_empty(&session_stop_path(session), &body)
            .await
    }
}

#[cfg(test)]
#[path = "video_play_tests.rs"]
mod tests;
