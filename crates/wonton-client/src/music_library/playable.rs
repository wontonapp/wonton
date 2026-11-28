//! 音乐页上能放的歌翻成媒体条目（ADR-0113）：点一首歌时，这一页上的歌按页面顺序就是要放的那一列。
//! 条目的 `source` 是这个音乐插件，核心按它找同一家起播；专辑页上和专辑歌手相同而省掉的署名补回来，
//! 锁屏和播放页要显示。

use wonton_protocol::media::{MediaItemRef, MediaKind};
use wonton_protocol::music::{MusicContent, MusicPageState, MusicPageView, TrackRow};

/// 这一页上能放的歌，按页面上的顺序；没有歌的页（首页、专辑墙、歌手）是空的。
pub(crate) fn playable(view: &MusicPageView, plugin_id: &str) -> Vec<MediaItemRef> {
    let MusicPageState::Ready { content, .. } = &view.state else {
        return Vec::new();
    };
    let item = |track: &TrackRow, artist: Option<&String>| media_item(plugin_id, track, artist);
    match content {
        MusicContent::Album { album } => album
            .discs
            .iter()
            .flat_map(|disc| &disc.tracks)
            .map(|track| item(track, album.album.artist.as_ref()))
            .collect(),
        MusicContent::Playlist { playlist } => {
            playlist.tracks.iter().map(|t| item(t, None)).collect()
        }
        MusicContent::Songs { songs } => songs.iter().map(|t| item(t, None)).collect(),
        MusicContent::Starred { starred } => starred.songs.iter().map(|t| item(t, None)).collect(),
        MusicContent::Search { results, .. } => {
            results.songs.iter().map(|t| item(t, None)).collect()
        }
        MusicContent::Home { .. }
        | MusicContent::Albums { .. }
        | MusicContent::Artists { .. }
        | MusicContent::Playlists { .. }
        | MusicContent::Genres { .. }
        | MusicContent::Artist { .. } => Vec::new(),
    }
}

fn media_item(plugin_id: &str, track: &TrackRow, album_artist: Option<&String>) -> MediaItemRef {
    MediaItemRef {
        id: track.id.clone(),
        kind: MediaKind::Audio,
        title: track.title.clone(),
        artist: track.artist.clone().or_else(|| album_artist.cloned()),
        album: track.album.clone(),
        artwork_url: None,
        duration_ms: Some(track.duration_ms),
        series: None,
        index: track.track,
        played: false,
        starred: track.starred,
        source: Some(plugin_id.to_owned()),
    }
}
