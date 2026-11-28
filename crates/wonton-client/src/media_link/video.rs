//! 影视的原生浏览（ADR-0113 第 10 节）：经核心转发取提供方的 `catalog`，翻成壳直接画的视图。
//!
//! 壳拿到的图片地址都是本机回环上的绝对地址（核心的封面路由，带要哪一张、多宽），壳的图片加载器直接取，
//! 不用再拼。能直接放的条目带好 `item`，壳拿它起播、建下载，不自己拼 `MediaItemRef`。

use futures_util::future::join_all;
use wonton_protocol::media::api::{MEDIA_API_PREFIX, item_artwork_path};
use wonton_protocol::media::catalog::{
    CatalogAction, CatalogActionResult, CatalogEntry, CatalogLibrary, CreditRole, EntriesQuery,
    EntryDetail, EntryPage, EntryType, Genres, Libraries, LibraryKind, MediaInfo,
    PATH_CATALOG_ACTIONS, PATH_CATALOG_LIBRARIES, Shelf, SortBy, entry_path, genres_path,
    shelf_path,
};
use wonton_protocol::media::provider::{ArtworkKind, ArtworkQuery, segment};
use wonton_protocol::media::{MediaFailure, MediaItemRef, MediaKind, MediaTrack};

use super::MediaLink;
use super::api::MediaApi;
use super::download::OfflineState;

/// 首页的几排，按这个顺序。
const HOME_SHELVES: [Shelf; 3] = [Shelf::Resume, Shelf::NextUp, Shelf::Latest];

/// 各种图要多宽（像素）。按手机 3 倍屏、海报墙三列估的，取大了提供方按原图给。
const POSTER_WIDTH: u32 = 480;
const THUMB_WIDTH: u32 = 800;
const BACKDROP_WIDTH: u32 = 1600;
const LOGO_WIDTH: u32 = 600;
const PERSON_WIDTH: u32 = 240;

/// 一张卡是什么：电影、剧、季、单集、其他视频或文件夹。决定点进去是详情还是海报墙。
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum VideoEntryKind {
    Movie,
    Series,
    Season,
    Episode,
    Video,
    Folder,
    /// 新版本的提供方给的新种类：当普通条目画，能不能放看 `item`。
    Other,
}

/// 海报墙、横排、详情里的一张卡。
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct VideoEntry {
    pub id: String,
    pub kind: VideoEntryKind,
    pub title: String,
    pub year: Option<u16>,
    pub series_id: Option<String>,
    pub series_title: Option<String>,
    pub season: Option<u32>,
    pub episode: Option<u32>,
    pub duration_ms: Option<u64>,
    pub position_ms: Option<u64>,
    /// 看到哪了（0–1）；没开始看、看完了为空。
    pub progress: Option<f32>,
    pub played: bool,
    pub favorite: bool,
    pub unplayed_count: Option<u32>,
    pub child_count: Option<u32>,
    pub community_rating: Option<f32>,
    pub official_rating: Option<String>,
    /// 竖版海报（单集是剧照）。
    pub poster: Option<String>,
    /// 横版图：缩略图，没有时单集用剧照、其余用背景图。
    pub landscape: Option<String>,
    pub backdrop: Option<String>,
    pub logo: Option<String>,
    /// 哪个媒体源插件给的；空着是核心按默认选的那一家，回传时照原样带上。
    pub source: Option<String>,
    /// 能直接放时的条目，拿去起播、建下载；剧、季、文件夹为空。
    pub item: Option<MediaItemRef>,
}

/// 片库装的是什么，壳按它挑图标。
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum VideoLibraryKind {
    Movies,
    Shows,
    Mixed,
    Other,
}

/// 一个片库（提供方里的“电影”“剧集”这类顶层分组）；点进去是这个库的海报墙。
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct VideoLibrary {
    pub id: String,
    pub title: String,
    pub kind: VideoLibraryKind,
    pub count: Option<u32>,
}

/// 首页的哪一排。
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum VideoShelfKind {
    Resume,
    NextUp,
    Latest,
}

/// 首页的一排卡。
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct VideoShelf {
    pub kind: VideoShelfKind,
    pub entries: Vec<VideoEntry>,
}

/// 影视首页：库和几排。排是尽量给的：某一排没取到，其余照常，没取到的原因在 `partial_failure`。
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct VideoHome {
    pub libraries: Vec<VideoLibrary>,
    /// 只有有内容的排，顺序是继续看、下一集、最近添加。
    pub shelves: Vec<VideoShelf>,
    pub partial_failure: Option<MediaFailure>,
}

/// 海报墙的排序。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, uniffi::Enum)]
pub enum VideoSort {
    #[default]
    Title,
    Added,
    Released,
    Rating,
}

/// 海报墙要什么。都空着是全部电影和剧。
#[derive(Debug, Clone, PartialEq, Default, uniffi::Record)]
pub struct VideoWallQuery {
    pub library: Option<String>,
    /// 列这一项下面的（剧的季、季的集、文件夹里的）。
    pub parent: Option<String>,
    pub kind: Option<VideoEntryKind>,
    pub genre: Option<String>,
    pub search: Option<String>,
    pub sort: VideoSort,
    pub unplayed_only: bool,
    /// 上一页回的 `cursor`；第一页为空。
    pub cursor: Option<String>,
    pub source: Option<String>,
}

/// 海报墙的一页。`cursor` 为空是没有下一页了。
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct VideoWall {
    pub entries: Vec<VideoEntry>,
    pub cursor: Option<String>,
    pub total: Option<u32>,
}

/// 演职员在这部片子里做什么。
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum VideoRole {
    Actor,
    Director,
    Writer,
    Producer,
    Other,
}

/// 详情里的一位演职员；`photo` 是本机回环上的头像地址。
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct VideoPerson {
    pub name: String,
    pub role: VideoRole,
    pub character: Option<String>,
    pub photo: Option<String>,
}

/// 片源的规格（清晰度、HDR、编码、声道、大小），详情里画成小标。
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct VideoMediaInfo {
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub hdr: bool,
    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
    pub audio_channels: Option<u8>,
    pub container: Option<String>,
    pub size: Option<u64>,
}

/// 详情页。剧带季（`children`）和该看的那一集（`next_up`），季带集。
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct VideoDetail {
    pub entry: VideoEntry,
    pub overview: Option<String>,
    pub tagline: Option<String>,
    pub genres: Vec<String>,
    pub studios: Vec<String>,
    pub premiered_at: Option<u64>,
    pub people: Vec<VideoPerson>,
    pub children: Vec<VideoEntry>,
    pub next_up: Option<VideoEntry>,
    pub audio: Vec<MediaTrack>,
    pub subtitles: Vec<MediaTrack>,
    pub media: Option<VideoMediaInfo>,
    /// 这台设备上的离线副本；没下过为空。
    pub offline: Option<OfflineState>,
}

/// 标看过、收藏的结果：没成的逐条列出，其余照常。
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct VideoActionResult {
    pub changed: u32,
    pub failed: Vec<VideoActionFailure>,
}

/// 一批动作里没成的那一条和原因。
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct VideoActionFailure {
    pub id: String,
    pub failure: MediaFailure,
}

/// 把提供方的条目翻成卡：`prefix` 是本机回环前缀，图片地址接在它后面。
pub(crate) struct Mapper<'a> {
    pub(crate) prefix: &'a str,
    pub(crate) source: Option<&'a str>,
}

impl Mapper<'_> {
    fn artwork(&self, id: &str, artwork: ArtworkKind, width: u32) -> String {
        let anchor = MediaItemRef {
            id: id.to_owned(),
            kind: MediaKind::Video,
            title: String::new(),
            artist: None,
            album: None,
            artwork_url: None,
            duration_ms: None,
            series: None,
            index: None,
            played: false,
            starred: false,
            source: self.source.map(str::to_owned),
        };
        let query = ArtworkQuery {
            artwork: Some(artwork),
            width: Some(width),
        };
        let path = item_artwork_path(&anchor, &query);
        let rest = path.strip_prefix(MEDIA_API_PREFIX).unwrap_or(&path);
        format!("{}{rest}", self.prefix)
    }

    pub(crate) fn entry(&self, entry: &CatalogEntry) -> VideoEntry {
        let has = |kind: &ArtworkKind| entry.artwork.contains(kind);
        let art = |kind: ArtworkKind, width| self.artwork(&entry.id, kind, width);
        let poster = has(&ArtworkKind::Primary).then(|| art(ArtworkKind::Primary, POSTER_WIDTH));
        let landscape = if has(&ArtworkKind::Thumb) {
            Some(art(ArtworkKind::Thumb, THUMB_WIDTH))
        } else if entry.entry_type == EntryType::Episode && has(&ArtworkKind::Primary) {
            Some(art(ArtworkKind::Primary, THUMB_WIDTH))
        } else {
            has(&ArtworkKind::Backdrop).then(|| art(ArtworkKind::Backdrop, THUMB_WIDTH))
        };
        VideoEntry {
            id: entry.id.clone(),
            kind: entry_kind(&entry.entry_type),
            title: entry.title.clone(),
            year: entry.year,
            series_id: entry.series_id.clone(),
            series_title: entry.series_title.clone(),
            season: entry.season,
            episode: entry.episode,
            duration_ms: entry.duration_ms,
            position_ms: entry.position_ms,
            progress: progress(entry),
            played: entry.played,
            favorite: entry.favorite,
            unplayed_count: entry.unplayed_count,
            child_count: entry.child_count,
            community_rating: entry.community_rating,
            official_rating: entry.official_rating.clone(),
            poster,
            landscape,
            backdrop: has(&ArtworkKind::Backdrop)
                .then(|| art(ArtworkKind::Backdrop, BACKDROP_WIDTH)),
            logo: has(&ArtworkKind::Logo).then(|| art(ArtworkKind::Logo, LOGO_WIDTH)),
            source: self.source.map(str::to_owned),
            item: entry.to_item(self.source),
        }
    }

    fn entries(&self, entries: &[CatalogEntry]) -> Vec<VideoEntry> {
        entries.iter().map(|e| self.entry(e)).collect()
    }

    pub(crate) fn detail(
        &self,
        detail: &EntryDetail,
        offline: Option<OfflineState>,
    ) -> VideoDetail {
        VideoDetail {
            entry: self.entry(&detail.entry),
            overview: detail.overview.clone(),
            tagline: detail.tagline.clone(),
            genres: detail.genres.clone(),
            studios: detail.studios.clone(),
            premiered_at: detail.premiered_at,
            people: detail
                .people
                .iter()
                .map(|p| VideoPerson {
                    name: p.name.clone(),
                    role: role(&p.role),
                    character: p.character.clone(),
                    photo: p
                        .id
                        .as_deref()
                        .filter(|_| p.artwork)
                        .map(|id| self.artwork(id, ArtworkKind::Primary, PERSON_WIDTH)),
                })
                .collect(),
            children: self.entries(&detail.children),
            next_up: detail.next_up.as_ref().map(|e| self.entry(e)),
            audio: detail.audio.clone(),
            subtitles: detail.subtitles.clone(),
            media: detail.media.as_ref().map(media_info),
            offline,
        }
    }
}

fn progress(entry: &CatalogEntry) -> Option<f32> {
    let (position, duration) = (entry.position_ms?, entry.duration_ms?);
    if entry.played || position == 0 || duration == 0 {
        return None;
    }
    #[allow(clippy::cast_precision_loss)]
    Some((position as f64 / duration as f64).clamp(0.0, 1.0) as f32)
}

fn entry_kind(kind: &EntryType) -> VideoEntryKind {
    match kind {
        EntryType::Movie => VideoEntryKind::Movie,
        EntryType::Series => VideoEntryKind::Series,
        EntryType::Season => VideoEntryKind::Season,
        EntryType::Episode => VideoEntryKind::Episode,
        EntryType::Video => VideoEntryKind::Video,
        EntryType::Folder => VideoEntryKind::Folder,
        EntryType::Other(_) => VideoEntryKind::Other,
    }
}

fn entry_type(kind: VideoEntryKind) -> Option<EntryType> {
    Some(match kind {
        VideoEntryKind::Movie => EntryType::Movie,
        VideoEntryKind::Series => EntryType::Series,
        VideoEntryKind::Season => EntryType::Season,
        VideoEntryKind::Episode => EntryType::Episode,
        VideoEntryKind::Video => EntryType::Video,
        VideoEntryKind::Folder => EntryType::Folder,
        VideoEntryKind::Other => return None,
    })
}

fn role(role: &CreditRole) -> VideoRole {
    match role {
        CreditRole::Actor => VideoRole::Actor,
        CreditRole::Director => VideoRole::Director,
        CreditRole::Writer => VideoRole::Writer,
        CreditRole::Producer => VideoRole::Producer,
        CreditRole::Other(_) => VideoRole::Other,
    }
}

fn library(library: &CatalogLibrary) -> VideoLibrary {
    VideoLibrary {
        id: library.id.clone(),
        title: library.title.clone(),
        kind: match library.kind {
            LibraryKind::Movies => VideoLibraryKind::Movies,
            LibraryKind::Shows => VideoLibraryKind::Shows,
            LibraryKind::Mixed => VideoLibraryKind::Mixed,
            LibraryKind::Other(_) => VideoLibraryKind::Other,
        },
        count: library.count,
    }
}

fn media_info(info: &MediaInfo) -> VideoMediaInfo {
    VideoMediaInfo {
        width: info.width,
        height: info.height,
        hdr: info.hdr,
        video_codec: info.video_codec.clone(),
        audio_codec: info.audio_codec.clone(),
        audio_channels: info.audio_channels,
        container: info.container.clone(),
        size: info.size,
    }
}

/// 认不得的排（新版本的提供方加的）不画：画成哪一排都是错的。
fn shelf_kind(shelf: &Shelf) -> Option<VideoShelfKind> {
    match shelf {
        Shelf::Resume => Some(VideoShelfKind::Resume),
        Shelf::NextUp => Some(VideoShelfKind::NextUp),
        Shelf::Latest => Some(VideoShelfKind::Latest),
        Shelf::Other(_) => None,
    }
}

pub(crate) fn wall_query(query: &VideoWallQuery) -> EntriesQuery {
    EntriesQuery {
        library: query.library.clone(),
        parent: query.parent.clone(),
        entry_type: query.kind.and_then(entry_type),
        genre: query.genre.clone(),
        q: query.search.clone().filter(|q| !q.trim().is_empty()),
        sort: Some(match query.sort {
            VideoSort::Title => SortBy::Title,
            VideoSort::Added => SortBy::Added,
            VideoSort::Released => SortBy::Released,
            VideoSort::Rating => SortBy::Rating,
        }),
        unplayed: query.unplayed_only.then_some(true),
        cursor: query.cursor.clone(),
    }
}

/// 核心转发 `catalog` 的地址：提供方的路径接在媒体接口后面，指定了哪一家就带 `source`（核心转发前去掉）。
pub(crate) fn catalog_path(path: &str, source: Option<&str>) -> String {
    let mut full = format!("{MEDIA_API_PREFIX}{path}");
    if let Some(source) = source {
        full.push(if path.contains('?') { '&' } else { '?' });
        full.push_str("source=");
        full.push_str(&segment(source));
    }
    full
}

/// 首页几排的结果拼起来：空排不给，没取到的记第一条原因。
pub(crate) fn home(
    mapper: &Mapper<'_>,
    libraries: &Libraries,
    shelves: Vec<(Shelf, Result<EntryPage, MediaFailure>)>,
) -> VideoHome {
    let mut partial_failure = None;
    let mut rows = Vec::new();
    for (shelf, page) in shelves {
        match page {
            Ok(page) if !page.entries.is_empty() => {
                if let Some(kind) = shelf_kind(&shelf) {
                    rows.push(VideoShelf {
                        kind,
                        entries: mapper.entries(&page.entries),
                    });
                }
            }
            Ok(_) => {}
            Err(failure) => {
                partial_failure.get_or_insert(failure);
            }
        }
    }
    VideoHome {
        libraries: libraries.libraries.iter().map(library).collect(),
        shelves: rows,
        partial_failure,
    }
}

impl MediaLink {
    async fn catalog_api(&self) -> Result<(MediaApi, String), MediaFailure> {
        let prefix = self.loopback().await?;
        Ok((self.api()?, prefix))
    }

    /// 首页：库必须取到（取不到整页报错），排尽量给。
    pub(crate) async fn video_home(
        &self,
        source: Option<String>,
    ) -> Result<VideoHome, MediaFailure> {
        let (api, prefix) = self.catalog_api().await?;
        let source = source.as_deref();
        let libraries_path = catalog_path(PATH_CATALOG_LIBRARIES, source);
        let libraries = api.get::<Libraries>(&libraries_path);
        let shelves = join_all(HOME_SHELVES.iter().map(|shelf| {
            let api = &api;
            async move {
                let page = api
                    .get::<EntryPage>(&catalog_path(&shelf_path(shelf, None), source))
                    .await;
                (shelf.clone(), page)
            }
        }));
        let (libraries, shelves) = futures_util::join!(libraries, shelves);
        let mapper = Mapper {
            prefix: &prefix,
            source,
        };
        Ok(home(&mapper, &libraries?, shelves))
    }

    pub(crate) async fn video_wall(
        &self,
        query: VideoWallQuery,
    ) -> Result<VideoWall, MediaFailure> {
        let (api, prefix) = self.catalog_api().await?;
        let source = query.source.as_deref();
        let page: EntryPage = api
            .get(&catalog_path(&wall_query(&query).path(), source))
            .await?;
        let mapper = Mapper {
            prefix: &prefix,
            source,
        };
        Ok(VideoWall {
            entries: mapper.entries(&page.entries),
            cursor: page.cursor,
            total: page.total,
        })
    }

    pub(crate) async fn video_detail(
        &self,
        id: String,
        source: Option<String>,
    ) -> Result<VideoDetail, MediaFailure> {
        let (api, prefix) = self.catalog_api().await?;
        let source = source.as_deref();
        let detail: EntryDetail = api.get(&catalog_path(&entry_path(&id), source)).await?;
        let mapper = Mapper {
            prefix: &prefix,
            source,
        };
        Ok(mapper.detail(&detail, self.downloads.offline(&id)))
    }

    pub(crate) async fn video_genres(
        &self,
        library: Option<String>,
        source: Option<String>,
    ) -> Result<Vec<String>, MediaFailure> {
        let path = catalog_path(&genres_path(library.as_deref()), source.as_deref());
        Ok(self.api()?.get::<Genres>(&path).await?.genres)
    }

    pub(crate) async fn video_act(
        &self,
        action: CatalogAction,
        source: Option<String>,
    ) -> Result<VideoActionResult, MediaFailure> {
        let path = catalog_path(PATH_CATALOG_ACTIONS, source.as_deref());
        let result: CatalogActionResult = self.api()?.post(&path, &action).await?;
        Ok(VideoActionResult {
            changed: result.changed,
            failed: result
                .failed
                .into_iter()
                .map(|f| VideoActionFailure {
                    id: f.id,
                    failure: f.failure,
                })
                .collect(),
        })
    }
}

#[cfg(test)]
#[path = "video_tests.rs"]
mod tests;
