use base64::Engine;
use serde::Deserialize;

const MAX_ARTWORK_BYTES: usize = 512 * 1024;
const MAX_MESSAGE_BYTES: usize = 768 * 1024;

/// 页面媒体播放状态；未知值不当作播放中。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, uniffi::Enum)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum PageMediaPlayback {
    None,
    Paused,
    Playing,
    #[serde(other)]
    Other,
}

/// 标准 Media Session 操作；未知操作不显示按钮。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, uniffi::Enum)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum PageMediaAction {
    Play,
    Pause,
    Stop,
    PreviousTrack,
    NextTrack,
    SeekBackward,
    SeekForward,
    SeekTo,
    #[serde(other)]
    Other,
}

/// 网页在状态变化时给系统播放器的快照，时间单位为毫秒。
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
#[non_exhaustive]
pub struct PageMediaState {
    pub playback: PageMediaPlayback,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub artwork: Option<Vec<u8>>,
    pub duration_ms: Option<u64>,
    pub position_ms: Option<u64>,
    pub rate: f64,
    pub actions: Vec<PageMediaAction>,
}

#[derive(Deserialize)]
struct Message {
    version: u32,
    playback: PageMediaPlayback,
    title: String,
    artist: String,
    album: String,
    artwork: Option<String>,
    duration_ms: Option<u64>,
    position_ms: Option<u64>,
    rate: f64,
    actions: Vec<PageMediaAction>,
}

pub(crate) fn script() -> String {
    include_str!("script.js").to_owned()
}

pub(crate) fn message(message: &str) -> Option<PageMediaState> {
    if message.len() > MAX_MESSAGE_BYTES {
        return None;
    }
    let message: Message = serde_json::from_str(message).ok()?;
    if message.version != 1 || !message.rate.is_finite() || message.rate <= 0.0 {
        return None;
    }
    let position_ms = message.position_ms.map(|position| {
        message
            .duration_ms
            .map_or(position, |duration| position.min(duration))
    });
    let artwork = match message.artwork {
        Some(encoded) => {
            if encoded.len() > MAX_ARTWORK_BYTES.div_ceil(3) * 4 {
                return None;
            }
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .ok()?;
            if bytes.len() > MAX_ARTWORK_BYTES {
                return None;
            }
            Some(bytes)
        }
        None => None,
    };
    let mut actions = Vec::with_capacity(message.actions.len());
    for action in message.actions {
        if action != PageMediaAction::Other && !actions.contains(&action) {
            actions.push(action);
        }
    }
    Some(PageMediaState {
        playback: message.playback,
        title: message.title,
        artist: message.artist,
        album: message.album,
        artwork,
        duration_ms: message.duration_ms,
        position_ms,
        rate: message.rate,
        actions,
    })
}

pub(crate) fn command_script(action: PageMediaAction, seek_ms: Option<u64>) -> String {
    let name = match action {
        PageMediaAction::Play => "play",
        PageMediaAction::Pause => "pause",
        PageMediaAction::Stop => "stop",
        PageMediaAction::PreviousTrack => "previoustrack",
        PageMediaAction::NextTrack => "nexttrack",
        PageMediaAction::SeekBackward => "seekbackward",
        PageMediaAction::SeekForward => "seekforward",
        PageMediaAction::SeekTo => "seekto",
        PageMediaAction::Other => return String::new(),
    };
    let seconds = seek_ms.map(|value| value as f64 / 1000.0);
    format!(
        "window.__wontonPageMedia?.command({name:?}, {});",
        serde_json::to_string(&seconds).expect("finite media seek position")
    )
}
