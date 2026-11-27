//! 原生相册页（ADR-0113 第 9 节）：客户端核按 [`super::library`] 的线上形状取回时间线、相册、搜索和详情，
//! 出这里的视图给壳；壳只画网格、手势和转场，用户操作经 [`PhotosAction`] 发回客户端核。
//!
//! 游标、按日分组、选择集都在客户端核；字节不进视图，壳按条目 id 和档位向客户端核要本机回环地址，
//! 交给系统的图片加载器和播放器。失败照搬核心回的 [`MediaFailure`]：壳按 `code` 出字，`message` 收进“详情”；
//! 装了不止一个相册插件又没定默认时是 `source_required`，壳让用户挑一个，带进 [`PhotosAction::Open`] 再来。
//! 服务端发来的取值认不出时落 `Other`（相册归属、媒体种类、失败码）；提供方新加的写操作这一版做不了，不进 `actions`。

use serde::{Deserialize, Serialize};

use super::library::{self, Album, AlbumOwner, PhotoAsset, PhotoDetail};
use super::{MediaFailure, MediaKind, Timestamp};

/// 相册页的整份视图。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PhotosViewModel {
    pub timeline: PhotoTimeline,
    pub albums: PhotoAlbums,
    pub people: PhotoPeople,
    /// 建相册、共享相册时能选的家人
    pub share_targets: PhotoShareTargets,
    /// 这个源接的写操作，壳只出这里有的按钮；接收藏时 `Favorite`、`Unfavorite` 都在。只读的源为空表；
    /// 还没取到或没取成时为空值，壳先不出写操作的按钮（原因看时间线的 `problem`）
    pub actions: Option<Vec<PhotoEditKind>>,
    /// 打开着的相册
    pub album: Option<PhotoAlbumView>,
    /// 搜索页的查询和结果；清掉搜索时为空
    pub search: Option<PhotoSearchView>,
    /// 打开着的大图页
    pub detail: Option<PhotoDetailView>,
    /// 选中的条目 id，按选中的先后
    pub selected: Vec<String>,
    /// 发出去还没回的写操作个数
    pub editing: u32,
    /// 最近一次写操作的结果，壳给一条轻提示（可撤的带撤销）；`DismissEditOutcome` 清掉
    pub last_edit: Option<PhotoEditOutcome>,
}

/// 时间线：按拍摄月分桶，新的在前。月只在打开时取条目。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PhotoTimeline {
    pub loading: bool,
    pub problem: Option<MediaFailure>,
    pub months: Vec<PhotoMonth>,
}

/// 时间线的一个月。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PhotoMonth {
    /// `YYYY-MM`
    pub month: String,
    pub count: u32,
    /// 取过（或正在取）这个月的条目
    pub opened: bool,
    /// 在取第一页
    pub loading: bool,
    /// 在取下一页
    pub loading_more: bool,
    /// 取第一页失败时没有条目；翻页失败时已取到的照留
    pub problem: Option<MediaFailure>,
    /// 按拍摄日分组，新的在前
    pub days: Vec<PhotoDay>,
    pub has_more: bool,
}

/// 一天的条目。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PhotoDay {
    /// `YYYY-MM-DD`，按 [`PhotosAction::Open`] 给的时差算
    pub date: String,
    pub tiles: Vec<PhotoTile>,
}

/// 网格里的一格。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PhotoTile {
    pub id: String,
    pub kind: MediaKind,
    pub taken_at: Timestamp,
    /// 宽比高；没有就按正方形排
    pub aspect: Option<f64>,
    pub duration_ms: Option<u64>,
    /// 实况/动态照片，动态部分按 [`PhotoRendition::Live`] 取
    pub live: bool,
    pub favorite: bool,
    /// 别人的条目写主人的本机用户名，壳据此标来源
    pub owner: Option<String>,
    /// ThumbHash 的 base64，缩略图到之前的占位
    pub thumbhash: Option<String>,
}

/// 相册和搜索结果的一页页条目（不按日分组：搜索按相关度排）。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PhotoPage {
    pub loading: bool,
    pub loading_more: bool,
    pub problem: Option<MediaFailure>,
    pub tiles: Vec<PhotoTile>,
    pub has_more: bool,
}

/// 相册列表，按归属分好：自己的、家庭相册、家人共享给我的。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PhotoAlbums {
    pub loading: bool,
    pub problem: Option<MediaFailure>,
    pub mine: Vec<PhotoAlbumRow>,
    pub household: Vec<PhotoAlbumRow>,
    /// 别的家人的，和认不出归属的
    pub shared_with_me: Vec<PhotoAlbumRow>,
}

/// 一个相册。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PhotoAlbumRow {
    pub id: String,
    pub title: String,
    pub count: u32,
    pub owner: PhotoAlbumOwner,
    /// 我建的共享出去了，或别人建的共享给我
    pub shared: bool,
    /// 封面那一条的 id，按 [`PhotoRendition::Thumbnail`] 取
    pub cover: Option<String>,
    /// 共享给的本机用户名，不含库主
    pub members: Vec<String>,
    pub updated_at: Option<Timestamp>,
}

/// 相册是谁的。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PhotoAlbumOwner {
    Me,
    Household,
    Person {
        username: String,
    },
    /// 新版本新加的归属，原值在里
    Other {
        raw: String,
    },
}

/// 打开着的相册：列表里有这一行就先带上，回包到了换成回包里的。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PhotoAlbumView {
    pub id: String,
    pub album: Option<PhotoAlbumRow>,
    pub page: PhotoPage,
}

/// 搜索条件；都空不能搜。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[serde(default)]
pub struct PhotoSearch {
    /// 画面描述
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub text: Option<String>,
    /// 人物 id（[`PhotoPerson::id`]）
    #[cfg_attr(feature = "uniffi", uniffi(default = []))]
    pub people: Vec<String>,
    /// 城市、地区或国家
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub place: Option<String>,
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub from: Option<Timestamp>,
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub to: Option<Timestamp>,
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub kind: Option<MediaKind>,
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub favorite: Option<bool>,
}

impl PhotoSearch {
    /// 一个条件都没写。
    pub fn is_empty(&self) -> bool {
        let blank =
            |value: &Option<String>| value.as_deref().is_none_or(|text| text.trim().is_empty());
        blank(&self.text)
            && self.people.is_empty()
            && blank(&self.place)
            && self.from.is_none()
            && self.to.is_none()
            && self.kind.is_none()
            && self.favorite.is_none()
    }

    /// 提供方的查询串；`cursor` 是翻页时带上的。
    pub fn to_query(&self, cursor: Option<&str>) -> library::SearchQuery {
        let trimmed = |value: &Option<String>| {
            value
                .as_deref()
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(str::to_owned)
        };
        library::SearchQuery {
            q: trimmed(&self.text),
            people: (!self.people.is_empty()).then(|| self.people.join(",")),
            place: trimmed(&self.place),
            from: self.from,
            to: self.to,
            kind: self.kind.clone(),
            favorite: self.favorite,
            cursor: cursor.map(str::to_owned),
        }
    }
}

/// 搜索页。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PhotoSearchView {
    pub query: PhotoSearch,
    pub page: PhotoPage,
}

/// 人物列表（搜索页按人筛）；头像按 id 向客户端核要回环地址。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PhotoPeople {
    pub loading: bool,
    pub problem: Option<MediaFailure>,
    pub people: Vec<PhotoPerson>,
}

/// 能共享给的本机家人：已加入的、不含自己。名单要“管理成员”权限才读得到，没有时 `problem` 是
/// `permission_denied`、名单为空，壳不出选人的地方。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PhotoShareTargets {
    pub loading: bool,
    pub problem: Option<MediaFailure>,
    pub people: Vec<PhotoShareTarget>,
}

/// 一个能共享给的家人；`username` 填进 `CreateAlbum.share_with`、`ShareAlbum.people`。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PhotoShareTarget {
    pub username: String,
    pub display_name: String,
}

/// 提供方认出的一个有名字的人（不是本机家人）。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PhotoPerson {
    pub id: String,
    pub name: String,
}

/// 大图页：网格里有这一格就先带上，详情另取。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PhotoDetailView {
    pub id: String,
    pub tile: Option<PhotoTile>,
    pub loading: bool,
    pub problem: Option<MediaFailure>,
    pub info: Option<PhotoInfo>,
}

/// 详情里网格没有的那些：EXIF、地点、人物。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PhotoInfo {
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub file_name: Option<String>,
    pub size: Option<u64>,
    pub description: Option<String>,
    pub camera: Option<String>,
    pub lens: Option<String>,
    /// 城市、地区、国家，按小到大
    pub place: Vec<String>,
    pub location: Option<PhotoLocation>,
    pub people: Vec<PhotoPerson>,
}

/// 经纬度（WGS84）。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PhotoLocation {
    pub lat: f64,
    pub lon: f64,
}

/// 取哪一档字节，同 [`library::Rendition`]。
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum PhotoRendition {
    /// 网格的小图
    Thumbnail,
    /// 大图页的预览
    Preview,
    /// 原件，存到手机、分享出去用
    Original,
    /// 视频的手机能播版本
    Playback,
    /// 实况照片的动态部分
    Live,
}

impl PhotoRendition {
    pub fn wire(self) -> library::Rendition {
        match self {
            Self::Thumbnail => library::Rendition::Thumbnail,
            Self::Preview => library::Rendition::Preview,
            Self::Original => library::Rendition::Original,
            Self::Playback => library::Rendition::Playback,
            Self::Live => library::Rendition::Live,
        }
    }
}

/// 原生页上本人点的写操作，同 [`library::LibraryAction`]；本人的、可撤，不发确认卡片（ADR-0024）。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PhotoEdit {
    Favorite {
        ids: Vec<String>,
        favorite: bool,
    },
    CreateAlbum {
        title: String,
        ids: Vec<String>,
        /// 建好就共享给的本机用户名
        share_with: Vec<String>,
    },
    AddToAlbum {
        album: String,
        ids: Vec<String>,
    },
    RemoveFromAlbum {
        album: String,
        ids: Vec<String>,
    },
    /// 共享给家人（本机用户名）
    ShareAlbum {
        album: String,
        people: Vec<String>,
    },
    /// 进回收站，不是永久删
    Trash {
        ids: Vec<String>,
    },
    /// 从回收站拿回来（撤销进回收站）
    Restore {
        ids: Vec<String>,
    },
}

impl PhotoEdit {
    pub fn kind(&self) -> PhotoEditKind {
        match self {
            Self::Favorite { favorite: true, .. } => PhotoEditKind::Favorite,
            Self::Favorite {
                favorite: false, ..
            } => PhotoEditKind::Unfavorite,
            Self::CreateAlbum { .. } => PhotoEditKind::CreateAlbum,
            Self::AddToAlbum { .. } => PhotoEditKind::AddToAlbum,
            Self::RemoveFromAlbum { .. } => PhotoEditKind::RemoveFromAlbum,
            Self::ShareAlbum { .. } => PhotoEditKind::ShareAlbum,
            Self::Trash { .. } => PhotoEditKind::Trash,
            Self::Restore { .. } => PhotoEditKind::Restore,
        }
    }

    /// 这次动到的条目（共享相册动的是人，不算）。
    pub fn ids(&self) -> &[String] {
        match self {
            Self::Favorite { ids, .. }
            | Self::CreateAlbum { ids, .. }
            | Self::AddToAlbum { ids, .. }
            | Self::RemoveFromAlbum { ids, .. }
            | Self::Trash { ids }
            | Self::Restore { ids } => ids,
            Self::ShareAlbum { .. } => &[],
        }
    }

    /// 发给提供方的动作。
    pub fn wire(&self) -> library::LibraryAction {
        use library::LibraryAction as Wire;
        match self.clone() {
            Self::Favorite { ids, favorite } => Wire::Favorite { ids, favorite },
            Self::CreateAlbum {
                title,
                ids,
                share_with,
            } => Wire::CreateAlbum {
                title,
                ids,
                share_with,
            },
            Self::AddToAlbum { album, ids } => Wire::AddToAlbum { album, ids },
            Self::RemoveFromAlbum { album, ids } => Wire::RemoveFromAlbum { album, ids },
            Self::ShareAlbum { album, people } => Wire::ShareAlbum { album, people },
            Self::Trash { ids } => Wire::Trash { ids },
            Self::Restore { ids } => Wire::Restore { ids },
        }
    }
}

/// 写操作的种类，壳按它出提示的字、决定给不给撤销。
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum PhotoEditKind {
    Favorite,
    Unfavorite,
    CreateAlbum,
    AddToAlbum,
    RemoveFromAlbum,
    ShareAlbum,
    Trash,
    Restore,
}

/// 一次写操作的结果。整个没成时 `problem` 有值、`done` 为空；部分成功时成了的在 `done`，没成的逐条在 `failed`。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PhotoEditOutcome {
    pub kind: PhotoEditKind,
    /// 成了的条目，撤销（如进回收站后恢复）就用它
    pub done: Vec<String>,
    /// 提供方报的改动条数
    pub changed: u32,
    pub failed: Vec<PhotoItemFailure>,
    /// 建相册、改相册时回的新相册
    pub album: Option<PhotoAlbumRow>,
    pub problem: Option<MediaFailure>,
}

/// 一条没成。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PhotoItemFailure {
    pub id: String,
    pub failure: MediaFailure,
}

/// 相册页的操作。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PhotosAction {
    /// 进相册页：取写操作表、时间线、相册和人物。`utc_offset_minutes` 是手机此刻的时差（东八区为 480），按日分组
    /// 用它。`source` 是用哪个相册插件（插件 id），空着由核心按主人定的默认挑；换了插件，已取的都清掉重取。
    Open {
        utc_offset_minutes: i32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "uniffi", uniffi(default = None))]
        source: Option<String>,
    },
    /// 重取写操作表、时间线、相册和人物；打开着的月、相册、搜索和大图页一起重取第一页
    Refresh,
    /// 取一个月的第一页（`YYYY-MM`）；已经取过的不重取
    OpenMonth {
        month: String,
    },
    LoadMoreMonth {
        month: String,
    },
    OpenAlbum {
        id: String,
    },
    LoadMoreAlbum,
    CloseAlbum,
    Search {
        query: PhotoSearch,
    },
    LoadMoreSearch,
    ClearSearch,
    OpenDetail {
        id: String,
    },
    CloseDetail,
    /// 选中或取消选中这些条目
    Select {
        ids: Vec<String>,
        selected: bool,
    },
    ClearSelection,
    Edit {
        edit: PhotoEdit,
    },
    DismissEditOutcome,
}

impl From<&PhotoAsset> for PhotoTile {
    fn from(asset: &PhotoAsset) -> Self {
        Self {
            id: asset.id.clone(),
            kind: asset.kind.clone(),
            taken_at: asset.taken_at,
            aspect: asset.aspect,
            duration_ms: asset.duration_ms,
            live: asset.live,
            favorite: asset.favorite,
            owner: asset.owner.clone(),
            thumbhash: asset.thumbhash.clone(),
        }
    }
}

impl From<&AlbumOwner> for PhotoAlbumOwner {
    fn from(owner: &AlbumOwner) -> Self {
        match owner {
            AlbumOwner::Me => Self::Me,
            AlbumOwner::Household => Self::Household,
            AlbumOwner::Person(username) => Self::Person {
                username: username.clone(),
            },
            AlbumOwner::Other(raw) => Self::Other { raw: raw.clone() },
        }
    }
}

impl From<&Album> for PhotoAlbumRow {
    fn from(album: &Album) -> Self {
        Self {
            id: album.id.clone(),
            title: album.title.clone(),
            count: album.count,
            owner: (&album.owner).into(),
            shared: album.shared,
            cover: album.cover.clone(),
            members: album.members.clone(),
            updated_at: album.updated_at,
        }
    }
}

impl From<&PhotoDetail> for PhotoInfo {
    fn from(detail: &PhotoDetail) -> Self {
        Self {
            width: detail.width,
            height: detail.height,
            file_name: detail.file_name.clone(),
            size: detail.size,
            description: detail.description.clone(),
            camera: detail.camera.clone(),
            lens: detail.lens.clone(),
            place: detail.place.clone(),
            location: detail.location.as_ref().map(|point| PhotoLocation {
                lat: point.lat,
                lon: point.lon,
            }),
            people: detail
                .people
                .iter()
                .map(|person| PhotoPerson {
                    id: person.id.clone(),
                    name: person.name.clone(),
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_query_trims_and_joins_people() {
        let search = PhotoSearch {
            text: Some("  海边  ".into()),
            people: vec!["p1".into(), "p2".into()],
            place: Some(" ".into()),
            ..PhotoSearch::default()
        };
        assert!(!search.is_empty());
        let query = search.to_query(Some("next"));
        assert_eq!(query.q.as_deref(), Some("海边"));
        assert_eq!(query.people.as_deref(), Some("p1,p2"));
        assert_eq!(query.place, None);
        assert_eq!(query.cursor.as_deref(), Some("next"));
        let blank = PhotoSearch {
            text: Some(" ".into()),
            ..PhotoSearch::default()
        };
        assert!(blank.is_empty());
    }

    #[test]
    fn edits_map_to_library_actions_and_kinds() {
        let edit = PhotoEdit::Favorite {
            ids: vec!["a1".into()],
            favorite: false,
        };
        assert_eq!(edit.kind(), PhotoEditKind::Unfavorite);
        assert_eq!(
            serde_json::to_value(edit.wire()).unwrap(),
            serde_json::json!({"kind": "favorite", "ids": ["a1"], "favorite": false})
        );
        let share = PhotoEdit::ShareAlbum {
            album: "al1".into(),
            people: vec!["mom".into()],
        };
        assert!(share.ids().is_empty());
    }

    #[test]
    fn album_owner_keeps_unknown_values() {
        let owner: AlbumOwner = serde_json::from_str("\"guest\"").unwrap();
        assert_eq!(
            PhotoAlbumOwner::from(&owner),
            PhotoAlbumOwner::Other {
                raw: "guest".into()
            }
        );
    }
}
