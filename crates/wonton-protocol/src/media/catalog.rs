//! 影视浏览（ADR-0113 第 10 节）：提供方按 `catalog` 操作给片库、首页几排、海报墙、详情和季集，
//! 核心原样转发，客户端核据此出原生影视页。
//!
//! `GET /items` 只给 Agent 找内容，字段不够画海报墙和详情；播放、下载仍按 [`MediaItemRef`] 走第 4 节的
//! 播放会话和第 3 节的下载任务，可播的条目用 [`CatalogEntry::to_item`] 换过去。图走契约里的封面路由
//! （[`super::provider::ArtworkQuery`]），回包里不带图片地址。
//!
//! 这里是核心与提供方之间、核心与客户端核之间同一份线上的形状（核心只转发，不改写），不过壳；壳看的
//! 视图由客户端核另出。服务端发来的取值枚举认不出时落 `Other`；[`CatalogAction`] 是提供方要执行的动作，
//! 认不得就没法执行，报错（同 [`super::QueueOp`]）。

use serde::{Deserialize, Serialize};

use super::library::ActionFailure;
use super::provider::{ArtworkKind, ArtworkQuery, segment};
use super::{MediaItemRef, MediaKind, MediaTrack, Timestamp, api, known_or_other};

/// 影视浏览各路径的前缀；提供方的路径和核心给客户端的 `/_wonton/api/media/catalog/…` 后半段相同。
pub const PATH_CATALOG: &str = "/catalog";

/// `GET`：这个人能看到的片库（电影、剧集……）。
pub const PATH_CATALOG_LIBRARIES: &str = "/catalog/libraries";
/// `GET`：海报墙和搜索，查询串见 [`EntriesQuery`]。
pub const PATH_CATALOG_ENTRIES: &str = "/catalog/entries";
/// `POST`：用户在原生页上的写操作，体是 [`CatalogAction`]。
pub const PATH_CATALOG_ACTIONS: &str = "/catalog/actions";

/// 首页的一排；`library` 省了就是全部片库。回 [`EntryPage`]。
pub fn shelf_path(shelf: &Shelf, library: Option<&str>) -> String {
    let base = format!("{PATH_CATALOG}/shelves/{}", segment(shelf.as_str()));
    match library {
        Some(library) => format!("{base}?library={}", segment(library)),
        None => base,
    }
}

/// 一条的详情；剧带季、季带集。回 [`EntryDetail`]。
pub fn entry_path(id: &str) -> String {
    format!("{PATH_CATALOG_ENTRIES}/{}", segment(id))
}

/// 片库里出现过的类型（分类页）。回 [`Genres`]。
pub fn genres_path(library: Option<&str>) -> String {
    match library {
        Some(library) => format!("{PATH_CATALOG}/genres?library={}", segment(library)),
        None => format!("{PATH_CATALOG}/genres"),
    }
}

/// 片库的类型，壳按它选海报墙的排法。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum LibraryKind {
    Movies,
    Shows,
    /// 混着放的（家庭录像、文件夹原样）
    Mixed,
    /// 新版本新加的类型，原值在里。
    Other(String),
}

known_or_other!(LibraryKind {
    Movies => "movies",
    Shows => "shows",
    Mixed => "mixed",
});

/// 一个片库。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct CatalogLibrary {
    pub id: String,
    pub title: String,
    pub kind: LibraryKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<u32>,
}

/// `GET /catalog/libraries` 的回包。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Libraries {
    pub libraries: Vec<CatalogLibrary>,
}

/// 条目在库里的层级。能直接放的只有 `Movie`、`Episode`、`Video`（[`EntryType::playable`]）。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum EntryType {
    Movie,
    Series,
    Season,
    Episode,
    /// 不属于电影和剧的单个视频（家庭录像、短片）
    Video,
    Folder,
    /// 新版本新加的层级，原值在里。
    Other(String),
}

known_or_other!(EntryType {
    Movie => "movie",
    Series => "series",
    Season => "season",
    Episode => "episode",
    Video => "video",
    Folder => "folder",
});

impl EntryType {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Movie => "movie",
            Self::Series => "series",
            Self::Season => "season",
            Self::Episode => "episode",
            Self::Video => "video",
            Self::Folder => "folder",
            Self::Other(raw) => raw,
        }
    }

    pub fn playable(&self) -> bool {
        matches!(self, Self::Movie | Self::Episode | Self::Video)
    }
}

/// 首页的一排。客户端发、提供方收：提供方不认识的回 `protocol`。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Shelf {
    /// 看了一半的，最近看的在前
    Resume,
    /// 追着的剧下一集没看的
    NextUp,
    /// 新加进库的，新的在前
    Latest,
    /// 新版本新加的一排，原值在里。
    Other(String),
}

known_or_other!(Shelf {
    Resume => "resume",
    NextUp => "next_up",
    Latest => "latest",
});

impl Shelf {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Resume => "resume",
            Self::NextUp => "next_up",
            Self::Latest => "latest",
            Self::Other(raw) => raw,
        }
    }
}

/// 海报墙的排序。`Title` 从 A 到 Z，其余新的、高的在前。
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub enum SortBy {
    #[default]
    Title,
    /// 加入片库的时间
    Added,
    /// 上映或首播时间
    Released,
    /// 社区评分
    Rating,
    /// 新版本新加的排法，原值在里。
    Other(String),
}

known_or_other!(SortBy {
    Title => "title",
    Added => "added",
    Released => "released",
    Rating => "rating",
});

impl SortBy {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Title => "title",
            Self::Added => "added",
            Self::Released => "released",
            Self::Rating => "rating",
            Self::Other(raw) => raw,
        }
    }
}

/// 海报墙的一格、详情页里的一季一集：画格子、角标和进度条要的都在这里，详情另取。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct CatalogEntry {
    pub id: String,
    pub entry_type: EntryType,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub year: Option<u16>,
    /// 单集、季所属的剧
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub series_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub series_title: Option<String>,
    /// 季号；季本身也带
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub season: Option<u32>,
    /// 集号
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub episode: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// 看到哪了；没看过或看完了不带
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position_ms: Option<u64>,
    /// 看完了；剧和季是“全看完了”
    #[serde(default)]
    pub played: bool,
    #[serde(default)]
    pub favorite: bool,
    /// 剧和季还有几集没看
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unplayed_count: Option<u32>,
    /// 剧有几季、季有几集、文件夹有几项
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child_count: Option<u32>,
    /// 0–10
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub community_rating: Option<f32>,
    /// 分级原文（`PG-13`、`TV-MA`）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub official_rating: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub added_at: Option<Timestamp>,
    /// 有哪几种图；没列的不去取，壳画占位。演职员照片按 [`Credit::id`] 走同一条封面路由
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artwork: Vec<ArtworkKind>,
}

impl CatalogEntry {
    /// 交给播放会话和下载任务的条目；不能直接放的（剧、季、文件夹）为空。`source` 填浏览的那一家
    /// （`catalog` 原样转发，回包里没有来源），起播、下载、取封面就落到同一家；海报地址是核心的封面路由。
    pub fn to_item(&self, source: Option<&str>) -> Option<MediaItemRef> {
        if !self.entry_type.playable() {
            return None;
        }
        let mut item = MediaItemRef {
            id: self.id.clone(),
            kind: MediaKind::Video,
            title: self.title.clone(),
            artist: None,
            album: None,
            artwork_url: None,
            duration_ms: self.duration_ms,
            series: self.series_title.clone(),
            index: self.episode,
            played: self.played,
            starred: self.favorite,
            source: source.map(str::to_owned),
        };
        if self.artwork.contains(&ArtworkKind::Primary) {
            item.artwork_url = Some(api::item_artwork_path(&item, &ArtworkQuery::default()));
        }
        Some(item)
    }
}

/// 一页条目；`cursor` 为空就是最后一页，同一组查询参数下稳定。`total` 是这组参数下一共几项，
/// 提供方算得出就带（海报墙的滚动条和字母索引用）。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct EntryPage {
    pub entries: Vec<CatalogEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<u32>,
}

/// `GET /catalog/entries` 的查询串；都可省。`parent` 是剧、季或文件夹的 id，列它下面的；
/// `entry_type` 省了时，提供方列这个片库最上一层（电影和剧，不含季和集）。
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq)]
pub struct EntriesQuery {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub library: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry_type: Option<EntryType>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub genre: Option<String>,
    /// 按片名、演职员搜
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub q: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort: Option<SortBy>,
    /// 只要没看完的
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unplayed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

impl EntriesQuery {
    /// 拼成 `/catalog/entries?…`：只带有值的参数，值按查询串转义；一个都没有就是 `/catalog/entries`。
    pub fn path(&self) -> String {
        let pairs = [
            ("library", self.library.clone()),
            ("parent", self.parent.clone()),
            (
                "entry_type",
                self.entry_type.as_ref().map(|t| t.as_str().to_owned()),
            ),
            ("genre", self.genre.clone()),
            ("q", self.q.clone()),
            ("sort", self.sort.as_ref().map(|s| s.as_str().to_owned())),
            ("unplayed", self.unplayed.map(|on| on.to_string())),
            ("cursor", self.cursor.clone()),
        ];
        let query: Vec<String> = pairs
            .into_iter()
            .filter_map(|(name, value)| value.map(|value| format!("{name}={}", segment(&value))))
            .collect();
        if query.is_empty() {
            PATH_CATALOG_ENTRIES.to_owned()
        } else {
            format!("{PATH_CATALOG_ENTRIES}?{}", query.join("&"))
        }
    }
}

/// 演职员的职务。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CreditRole {
    Actor,
    Director,
    Writer,
    Producer,
    /// 新版本新加的职务，原值在里。
    Other(String),
}

known_or_other!(CreditRole {
    Actor => "actor",
    Director => "director",
    Writer => "writer",
    Producer => "producer",
});

/// 一位演职员。`id` 有就能按它走封面路由取照片。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Credit {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub name: String,
    pub role: CreditRole,
    /// 演员演的角色
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub character: Option<String>,
    /// 有照片
    #[serde(default)]
    pub artwork: bool,
}

/// 片源的规格，详情页的“4K”“HDR”“5.1”角标用；以播放计划为准，这里只是展示。
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq)]
pub struct MediaInfo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    #[serde(default)]
    pub hdr: bool,
    /// 编解码名（`hevc`、`h264`）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_codec: Option<String>,
    /// 默认音轨的编解码名和声道数
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_codec: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_channels: Option<u8>,
    /// 扩展名（`mkv`、`mp4`）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
}

/// `GET /catalog/entries/{id}` 的回包。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct EntryDetail {
    pub entry: CatalogEntry,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overview: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tagline: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub genres: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub studios: Vec<String>,
    /// 上映或首播日（Unix 毫秒，当天 0 点 UTC）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub premiered_at: Option<Timestamp>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub people: Vec<Credit>,
    /// 剧 → 各季，季 → 各集，按季号、集号排；别的为空
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<CatalogEntry>,
    /// 剧的播放键放哪一集：看了一半的那集，否则下一集没看的，都看完了就第一集
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_up: Option<CatalogEntry>,
    /// 能直接放的条目才有；播放前先选的音轨和字幕，起播后以播放计划为准
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub audio: Vec<MediaTrack>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subtitles: Vec<MediaTrack>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media: Option<MediaInfo>,
}

/// `GET /catalog/genres` 的回包，按名字排。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Genres {
    pub genres: Vec<String>,
}

/// 用户在原生页上的写操作。本人点的、可撤，不发确认卡片（ADR-0024）。剧和季标已看，提供方把下面各集一起标。
/// 提供方认不得就回 `protocol`，不静默忽略。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum CatalogAction {
    SetPlayed { ids: Vec<String>, played: bool },
    Favorite { ids: Vec<String>, favorite: bool },
}

/// 写操作的结果。`failed` 是没成的那几条和原因，成了的不列。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct CatalogActionResult {
    pub changed: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failed: Vec<ActionFailure>,
}

#[cfg(test)]
#[path = "catalog_tests.rs"]
mod tests;
