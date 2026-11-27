//! 页里的条目。id 都是提供方的原值，只回传给核心，壳不解析。
//!
//! 封面只给 `cover`（提供方的封面 id），壳拿它和想要的边长调 `ClientCore::music_cover` 换图片字节；
//! 没有封面时为空，壳画占位。

use serde::{Deserialize, Serialize};

/// 页的内容，和 [`super::MusicPage`] 一一对应。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "content", rename_all = "snake_case")]
pub enum MusicContent {
    Home {
        home: MusicHome,
    },
    Albums {
        albums: Vec<AlbumTile>,
    },
    Artists {
        artists: Vec<ArtistRow>,
    },
    Playlists {
        playlists: Vec<PlaylistRow>,
    },
    Songs {
        songs: Vec<TrackRow>,
    },
    Starred {
        starred: MusicStarred,
    },
    Genres {
        genres: Vec<GenreRow>,
    },
    Album {
        album: AlbumDetail,
    },
    Artist {
        artist: ArtistDetail,
    },
    Playlist {
        playlist: PlaylistDetail,
    },
    /// 搜索页；`query` 是这批结果对应的词，壳拿它和输入框比，不一样就还在搜。
    Search {
        query: String,
        results: MusicSearch,
    },
}

/// 首页的几排；空的那排壳不画。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct MusicHome {
    pub recently_added: Vec<AlbumTile>,
    pub recently_played: Vec<AlbumTile>,
    pub most_played: Vec<AlbumTile>,
    pub playlists: Vec<PlaylistRow>,
}

/// 一张专辑，网格和列表都用它。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct AlbumTile {
    pub id: String,
    pub title: String,
    pub artist: Option<String>,
    pub artist_id: Option<String>,
    pub year: Option<u16>,
    pub cover: Option<String>,
    pub starred: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ArtistRow {
    pub id: String,
    pub name: String,
    pub album_count: u32,
    pub cover: Option<String>,
    pub starred: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PlaylistRow {
    pub id: String,
    pub name: String,
    /// 不是自己建的才有：显示“来自某某”。
    pub owner: Option<String>,
    pub song_count: u32,
    pub duration_ms: u64,
    pub cover: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct GenreRow {
    pub name: String,
    pub album_count: u32,
    pub song_count: u32,
}

/// 一首歌。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct TrackRow {
    pub id: String,
    pub title: String,
    /// 在专辑页里和专辑歌手相同时为空：壳只在有值时显示第二行。
    pub artist: Option<String>,
    pub artist_id: Option<String>,
    pub album: Option<String>,
    pub album_id: Option<String>,
    pub track: Option<u32>,
    pub disc: Option<u32>,
    pub duration_ms: u64,
    pub cover: Option<String>,
    pub starred: bool,
}

/// 专辑页。多张碟时 `discs` 每碟一段，只有一张碟时只有一段、`title` 为空。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct AlbumDetail {
    pub album: AlbumTile,
    pub genre: Option<String>,
    pub song_count: u32,
    pub duration_ms: u64,
    pub discs: Vec<DiscSection>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct DiscSection {
    /// 碟号；只有一张碟时为空。
    pub disc: Option<u32>,
    /// OpenSubsonic 的碟名，没有时为空，壳写“第 N 张”。
    pub title: Option<String>,
    pub tracks: Vec<TrackRow>,
}

/// 歌手页：专辑按年份从新到旧。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ArtistDetail {
    pub artist: ArtistRow,
    pub albums: Vec<AlbumTile>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PlaylistDetail {
    pub playlist: PlaylistRow,
    pub comment: Option<String>,
    pub tracks: Vec<TrackRow>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct MusicStarred {
    pub artists: Vec<ArtistRow>,
    pub albums: Vec<AlbumTile>,
    pub songs: Vec<TrackRow>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct MusicSearch {
    pub artists: Vec<ArtistRow>,
    pub albums: Vec<AlbumTile>,
    pub songs: Vec<TrackRow>,
}

/// 一首歌的歌词。`synced` 时每行有起点，壳跟着播放位置滚动、点行跳过去；不同步时只有文字。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct MusicLyrics {
    pub synced: bool,
    pub lines: Vec<LyricLine>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct LyricLine {
    pub start_ms: Option<u64>,
    pub text: String,
}

/// 取歌词的结果：没有歌词和取失败分开，壳没有歌词时只显示封面，失败时给重试。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum MusicLyricsState {
    Found { lyrics: MusicLyrics },
    Missing,
    Failed { problem: super::MusicProblem },
}
