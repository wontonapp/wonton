//! OpenSubsonic 响应里用到的字段（<https://opensubsonic.netlify.app/docs/responses/>）。只写我们读的，
//! 其余字段忽略；列表字段在空库时上游会整个省掉，所以都带 `default`。时长是秒。

use serde::Deserialize;

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub(super) struct Album {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) artist: Option<String>,
    pub(super) artist_id: Option<String>,
    pub(super) cover_art: Option<String>,
    pub(super) song_count: u32,
    pub(super) duration: u64,
    pub(super) year: Option<u16>,
    pub(super) genre: Option<String>,
    /// 收藏的时刻；有值就是收藏了。
    pub(super) starred: Option<String>,
    #[serde(rename = "song")]
    pub(super) songs: Vec<Song>,
    pub(super) disc_titles: Vec<DiscTitle>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub(super) struct DiscTitle {
    pub(super) disc: u32,
    pub(super) title: String,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub(super) struct Song {
    pub(super) id: String,
    pub(super) title: String,
    pub(super) artist: Option<String>,
    pub(super) artist_id: Option<String>,
    pub(super) album: Option<String>,
    pub(super) album_id: Option<String>,
    pub(super) track: Option<u32>,
    pub(super) disc_number: Option<u32>,
    pub(super) duration: u64,
    pub(super) cover_art: Option<String>,
    pub(super) starred: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub(super) struct Artist {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) cover_art: Option<String>,
    pub(super) album_count: u32,
    pub(super) starred: Option<String>,
    #[serde(rename = "album")]
    pub(super) albums: Vec<Album>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub(super) struct Playlist {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) owner: Option<String>,
    /// OpenSubsonic：当前用户改不了这张歌单（不是他的）。
    pub(super) readonly: Option<bool>,
    pub(super) comment: Option<String>,
    pub(super) song_count: u32,
    pub(super) duration: u64,
    pub(super) cover_art: Option<String>,
    #[serde(rename = "entry")]
    pub(super) entries: Vec<Song>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub(super) struct Genre {
    pub(super) value: String,
    pub(super) song_count: u32,
    pub(super) album_count: u32,
}

/// `getAlbumList2` 的 `albumList2`。
#[derive(Deserialize, Default)]
#[serde(default)]
pub(super) struct AlbumList {
    #[serde(rename = "album")]
    pub(super) albums: Vec<Album>,
}

/// `getArtists` 的 `artists`：按首字母分组。
#[derive(Deserialize, Default)]
#[serde(default)]
pub(super) struct ArtistIndexes {
    pub(super) index: Vec<ArtistIndex>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub(super) struct ArtistIndex {
    #[serde(rename = "artist")]
    pub(super) artists: Vec<Artist>,
}

/// `getPlaylists` 的 `playlists`。
#[derive(Deserialize, Default)]
#[serde(default)]
pub(super) struct Playlists {
    #[serde(rename = "playlist")]
    pub(super) playlists: Vec<Playlist>,
}

/// `getGenres` 的 `genres`。
#[derive(Deserialize, Default)]
#[serde(default)]
pub(super) struct Genres {
    #[serde(rename = "genre")]
    pub(super) genres: Vec<Genre>,
}

/// `search3` 的 `searchResult3` 与 `getStarred2` 的 `starred2` 同形。
#[derive(Deserialize, Default)]
#[serde(default)]
pub(super) struct Found {
    #[serde(rename = "artist")]
    pub(super) artists: Vec<Artist>,
    #[serde(rename = "album")]
    pub(super) albums: Vec<Album>,
    #[serde(rename = "song")]
    pub(super) songs: Vec<Song>,
}

/// `getLyricsBySongId` 的 `lyricsList`（OpenSubsonic `songLyrics` 扩展）。
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub(super) struct LyricsList {
    pub(super) structured_lyrics: Vec<StructuredLyrics>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub(super) struct StructuredLyrics {
    pub(super) synced: bool,
    pub(super) line: Vec<Line>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub(super) struct Line {
    /// 毫秒，只有同步歌词有。
    pub(super) start: Option<u64>,
    pub(super) value: String,
}
