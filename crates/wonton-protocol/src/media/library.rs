//! 相册浏览（ADR-0113 第 9 节）：提供方按 `library` 操作给时间线、相册、搜索和字节，核心原样转发，
//! 客户端核据此出原生相册页。
//!
//! 这里是核心与提供方之间、核心与客户端核之间同一份线上的形状（核心只转发，不改写），不过壳；
//! 壳看的视图由客户端核另出。服务端发来的取值枚举认不出时落 `Other`；[`LibraryAction`] 是提供方要
//! 执行的动作，认不得就没法执行，报错（同 [`super::QueueOp`]）。
//!
//! 失败一律回 [`super::MediaFailure`] 信封，码按 ADR-0113 第 9 节的表：请求本身不合法才是 `protocol`；
//! 这个人看不到或改不了某一条是 `permission_denied`，条目不在了是 `media_gone`，上游把这两种说成 400 的，
//! 提供方也要照这样归类。写操作逐条判断，做不了的那几条进 [`LibraryActionResult::failed`]，不连累整批。

use serde::{Deserialize, Serialize};

use super::provider::segment;
use super::{MediaKind, Timestamp, known_or_other};

/// 相册浏览各路径的前缀；提供方的路径和核心给客户端的 `/_wonton/api/media/library/…` 后半段相同。
/// `GET` 它本身回 [`LibraryFeatures`]：这个提供方接哪些写操作。
pub const PATH_LIBRARY: &str = "/library";

/// `GET`：按月分桶的时间线，新的在前。
pub const PATH_LIBRARY_TIMELINE: &str = "/library/timeline";
/// `GET`：相册列表。
pub const PATH_LIBRARY_ALBUMS: &str = "/library/albums";
/// `GET`：搜索，查询串见 [`SearchQuery`]。
pub const PATH_LIBRARY_SEARCH: &str = "/library/search";
/// `GET`：人物列表（搜索页按人筛）。
pub const PATH_LIBRARY_PEOPLE: &str = "/library/people";
/// `POST`：用户在原生页上的写操作，体是 [`LibraryAction`]。
pub const PATH_LIBRARY_ACTIONS: &str = "/library/actions";

/// 一个月的条目：`month` 是 `YYYY-MM`（按拍摄时间、提供方所在时区）。
pub fn timeline_bucket_path(month: &str) -> String {
    format!("{PATH_LIBRARY_TIMELINE}/{}", segment(month))
}

/// 一个相册和它的条目。
pub fn album_path(id: &str) -> String {
    format!("{PATH_LIBRARY_ALBUMS}/{}", segment(id))
}

/// 一条的详情（EXIF、地点、人物）。
pub fn library_item_path(id: &str) -> String {
    format!("{PATH_LIBRARY}/items/{}", segment(id))
}

/// 一个人物的头像（提供方裁好的脸）。
pub fn person_thumbnail_path(id: &str) -> String {
    format!("{PATH_LIBRARY_PEOPLE}/{}/thumbnail", segment(id))
}

/// 一条的字节，`Range` 原样过。条件请求命中回 304、`Range` 越界回 416，都不是失败。
pub fn library_content_path(id: &str, rendition: &Rendition) -> String {
    format!(
        "{}/content?rendition={}",
        library_item_path(id),
        segment(rendition.as_str())
    )
}

/// `GET /library` 的回包：这个提供方接哪些写操作。只读的源回空表，壳不出对应按钮；
/// 发来不在表里的动作，提供方回 `protocol`。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct LibraryFeatures {
    #[serde(default)]
    pub actions: Vec<LibraryActionKind>,
}

/// 写操作的种类，同 [`LibraryAction`] 的 `kind`。新版本新加的种类落 `Other`。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum LibraryActionKind {
    Favorite,
    CreateAlbum,
    AddToAlbum,
    RemoveFromAlbum,
    ShareAlbum,
    Trash,
    Restore,
    Other(String),
}

known_or_other!(LibraryActionKind {
    Favorite => "favorite",
    CreateAlbum => "create_album",
    AddToAlbum => "add_to_album",
    RemoveFromAlbum => "remove_from_album",
    ShareAlbum => "share_album",
    Trash => "trash",
    Restore => "restore",
});

/// 时间线的一个月。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct TimelineBucket {
    /// `YYYY-MM`
    pub month: String,
    pub count: u32,
}

/// `GET /library/timeline` 的回包。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Timeline {
    pub buckets: Vec<TimelineBucket>,
}

/// 网格里的一格：画网格、排版、角标要的字段都在这里，详情另取。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct PhotoAsset {
    pub id: String,
    /// `photo` 或 `video`
    pub kind: MediaKind,
    pub taken_at: Timestamp,
    /// 宽比高（已按方向转正）；没有就空着，壳按正方形排
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aspect: Option<f64>,
    /// 视频的时长；实况照片的动态部分不算
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// 实况/动态照片：动态部分按 [`Rendition::Live`] 取
    #[serde(default)]
    pub live: bool,
    #[serde(default)]
    pub favorite: bool,
    /// 不是本人的条目（家人共享给他的、共享相册里别人加的）写主人的本机用户名；本人的为空
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    /// 原图解码前的占位图（ThumbHash 的 base64）；没有就空着，壳画纯色
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumbhash: Option<String>,
}

/// 一页条目；`cursor` 为空就是最后一页，同一组查询参数下稳定。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct AssetPage {
    pub assets: Vec<PhotoAsset>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

/// 大图页的详情。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct PhotoDetail {
    pub asset: PhotoAsset,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub camera: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lens: Option<String>,
    /// 城市、地区、国家里有的，按小到大
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub place: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<GeoPoint>,
    /// 认出的、有名字的人
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub people: Vec<PersonRef>,
}

/// 经纬度（WGS84）。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct GeoPoint {
    pub lat: f64,
    pub lon: f64,
}

/// 照片里认出的一个人（提供方的人物，不是本机家人）。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct PersonRef {
    pub id: String,
    pub name: String,
}

/// `GET /library/people` 的回包；只列有名字的，头像按 [`person_thumbnail_path`] 取。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct People {
    pub people: Vec<PersonEntry>,
}

/// 人物列表的一项。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct PersonEntry {
    pub id: String,
    pub name: String,
}

/// 相册是谁的。家庭账号是共享相册的库主（ADR-0113 第 6 节），壳单列“家庭相册”；哪个上游账号算家庭账号
/// 由提供方的配置定，没配就不会出现 `Household`。
#[derive(Debug, Clone, PartialEq)]
pub enum AlbumOwner {
    Me,
    Household,
    /// 别的家人，值是本机用户名
    Person(String),
    /// 新版本新加的归属，原值在里。
    Other(String),
}

impl Serialize for AlbumOwner {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Me => serializer.serialize_str("me"),
            Self::Household => serializer.serialize_str("household"),
            Self::Person(name) => serializer.serialize_str(&format!("person:{name}")),
            Self::Other(raw) => serializer.serialize_str(raw),
        }
    }
}

impl<'de> Deserialize<'de> for AlbumOwner {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Ok(match raw.as_str() {
            "me" => Self::Me,
            "household" => Self::Household,
            _ => match raw.strip_prefix("person:") {
                Some(name) if !name.is_empty() => Self::Person(name.to_owned()),
                _ => Self::Other(raw),
            },
        })
    }
}

/// 一个相册。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Album {
    pub id: String,
    pub title: String,
    pub count: u32,
    pub owner: AlbumOwner,
    /// 共享给了别人（我建的）或别人共享给我（别人建的）
    pub shared: bool,
    /// 封面那一条的 id
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover: Option<String>,
    /// 除库主外能看这个相册的本机用户名；可能含正在看的这个人
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub members: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<Timestamp>,
}

/// `GET /library/albums` 的回包。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Albums {
    pub albums: Vec<Album>,
}

/// `GET /library/albums/{id}` 的回包：相册本身加一页条目。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct AlbumPage {
    pub album: Album,
    #[serde(flatten)]
    pub page: AssetPage,
}

/// 搜索的查询串（[`SearchQuery::path`]）；都可省，全省就是报错（提供方回 `protocol`）。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct SearchQuery {
    /// 画面描述（按内容搜）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub q: Option<String>,
    /// 人物 id，逗号分隔
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub people: Option<String>,
    /// 地名（城市、地区或国家）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub place: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<Timestamp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<Timestamp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<MediaKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub favorite: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

/// 取哪一档字节。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Rendition {
    /// 网格用的小图（短边约 250 像素）
    Thumbnail,
    /// 大图页用的预览（短边约 1440 像素）
    Preview,
    /// 原件，存到手机、分享出去用
    Original,
    /// 视频的手机能播版本（提供方转码过的；没有转码就是原件）
    Playback,
    /// 实况/动态照片的动态部分
    Live,
    /// 新版本新加的档位，原值在里。
    Other(String),
}

known_or_other!(Rendition {
    Thumbnail => "thumbnail",
    Preview => "preview",
    Original => "original",
    Playback => "playback",
    Live => "live",
});

impl SearchQuery {
    /// 拼成 `/library/search?…`：只带有值的参数，值按查询串转义。
    pub fn path(&self) -> String {
        let kind = self.kind.as_ref().map(|kind| match kind {
            MediaKind::Photo => "photo".to_owned(),
            MediaKind::Video => "video".to_owned(),
            MediaKind::Audio => "audio".to_owned(),
            MediaKind::Other(raw) => raw.clone(),
        });
        let pairs = [
            ("q", self.q.clone()),
            ("people", self.people.clone()),
            ("place", self.place.clone()),
            ("from", self.from.map(|at| at.to_string())),
            ("to", self.to.map(|at| at.to_string())),
            ("kind", kind),
            ("favorite", self.favorite.map(|on| on.to_string())),
            ("cursor", self.cursor.clone()),
        ];
        let query: Vec<String> = pairs
            .into_iter()
            .filter_map(|(name, value)| value.map(|value| format!("{name}={}", segment(&value))))
            .collect();
        if query.is_empty() {
            return PATH_LIBRARY_SEARCH.to_owned();
        }
        format!("{PATH_LIBRARY_SEARCH}?{}", query.join("&"))
    }
}

impl Rendition {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Thumbnail => "thumbnail",
            Self::Preview => "preview",
            Self::Original => "original",
            Self::Playback => "playback",
            Self::Live => "live",
            Self::Other(raw) => raw,
        }
    }
}

/// 用户在原生页上的写操作。本人点的、可撤，不发确认卡片（ADR-0024）；Agent 走插件自己的 MCP 工具。
/// 提供方认不得、或不在它 [`LibraryFeatures`] 里，就回 `protocol`，不静默忽略。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum LibraryAction {
    Favorite {
        ids: Vec<String>,
        favorite: bool,
    },
    CreateAlbum {
        title: String,
        #[serde(default)]
        ids: Vec<String>,
        /// 建好就共享给的本机用户名
        #[serde(default)]
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
    /// 共享给家人；`people` 是本机用户名，已在的不重复加
    ShareAlbum {
        album: String,
        people: Vec<String>,
    },
    /// 进回收站，不是永久删
    Trash {
        ids: Vec<String>,
    },
    Restore {
        ids: Vec<String>,
    },
}

impl LibraryAction {
    pub fn kind(&self) -> LibraryActionKind {
        match self {
            Self::Favorite { .. } => LibraryActionKind::Favorite,
            Self::CreateAlbum { .. } => LibraryActionKind::CreateAlbum,
            Self::AddToAlbum { .. } => LibraryActionKind::AddToAlbum,
            Self::RemoveFromAlbum { .. } => LibraryActionKind::RemoveFromAlbum,
            Self::ShareAlbum { .. } => LibraryActionKind::ShareAlbum,
            Self::Trash { .. } => LibraryActionKind::Trash,
            Self::Restore { .. } => LibraryActionKind::Restore,
        }
    }
}

/// 写操作的结果。`failed` 是没成的那几条和原因（别人的照片、不在了的条目），成了的不列；上游整批
/// 全有或全无时，提供方先把做不了的挑出来进 `failed`，再做其余的。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct LibraryActionResult {
    pub changed: u32,
    /// 建相册、改相册时回新的相册
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub album: Option<Album>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failed: Vec<ActionFailure>,
}

/// 一条没成。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ActionFailure {
    pub id: String,
    pub failure: super::MediaFailure,
}

#[cfg(test)]
mod tests {
    use super::super::{MediaFailure, MediaFailureCode};
    use super::*;

    fn asset() -> PhotoAsset {
        PhotoAsset {
            id: "a1".into(),
            kind: MediaKind::Photo,
            taken_at: 1_700_000_000_000,
            aspect: Some(4.0 / 3.0),
            duration_ms: None,
            live: true,
            favorite: false,
            owner: Some("mom".into()),
            thumbhash: Some("1QcSHQRnh493V4dIh4eXh1h4kJUI".into()),
        }
    }

    fn album() -> Album {
        Album {
            id: "al1".into(),
            title: "海边".into(),
            count: 14,
            owner: AlbumOwner::Household,
            shared: true,
            cover: Some("a1".into()),
            members: vec!["mom".into(), "dad".into()],
            updated_at: Some(1_700_000_000_000),
        }
    }

    fn roundtrip<T>(value: &T) -> T
    where
        T: Serialize + serde::de::DeserializeOwned,
    {
        serde_json::from_value(serde_json::to_value(value).unwrap()).unwrap()
    }

    #[test]
    fn paths_join_expected() {
        assert_eq!(timeline_bucket_path("2026-11"), "/library/timeline/2026-11");
        assert_eq!(album_path("al1"), "/library/albums/al1");
        assert_eq!(library_item_path("a1"), "/library/items/a1");
        assert_eq!(person_thumbnail_path("p1"), "/library/people/p1/thumbnail");
        assert_eq!(
            library_content_path("a1", &Rendition::Live),
            "/library/items/a1/content?rendition=live"
        );
        assert_eq!(album_path("a/b c"), "/library/albums/a%2Fb%20c");
        assert_eq!(album_path(".."), "/library/albums/%2E%2E");
        assert_eq!(
            library_content_path("../x", &Rendition::Other("h dr?".into())),
            "/library/items/..%2Fx/content?rendition=h%20dr%3F"
        );
        for path in [
            PATH_LIBRARY_TIMELINE,
            PATH_LIBRARY_ALBUMS,
            PATH_LIBRARY_SEARCH,
            PATH_LIBRARY_PEOPLE,
            PATH_LIBRARY_ACTIONS,
        ] {
            assert!(path.starts_with(PATH_LIBRARY));
        }
    }

    #[test]
    fn asset_pages_roundtrip_and_omit_empty_fields() {
        let page = AssetPage {
            assets: vec![asset()],
            cursor: Some("next".into()),
        };
        assert_eq!(roundtrip(&page), page);

        let bare: PhotoAsset = serde_json::from_value(serde_json::json!({
            "id": "a2", "kind": "video", "taken_at": 1
        }))
        .unwrap();
        assert!(!bare.live && !bare.favorite && bare.owner.is_none());
        let json = serde_json::to_value(&bare).unwrap();
        assert!(json.get("aspect").is_none() && json.get("thumbhash").is_none());
    }

    #[test]
    fn album_page_flattens_assets_next_to_album() {
        let page = AlbumPage {
            album: album(),
            page: AssetPage {
                assets: vec![asset()],
                cursor: None,
            },
        };
        let json = serde_json::to_value(&page).unwrap();
        assert_eq!(json["assets"][0]["id"], "a1");
        assert_eq!(json["album"]["owner"], "household");
        assert_eq!(roundtrip(&page), page);
    }

    #[test]
    fn album_owner_reads_people_and_falls_to_other() {
        let owner: AlbumOwner = serde_json::from_str("\"person:mom\"").unwrap();
        assert_eq!(owner, AlbumOwner::Person("mom".into()));
        assert_eq!(serde_json::to_value(&owner).unwrap(), "person:mom");
        for raw in ["\"guest\"", "\"person:\""] {
            let owner: AlbumOwner = serde_json::from_str(raw).unwrap();
            assert!(matches!(owner, AlbumOwner::Other(_)));
            assert_eq!(serde_json::to_string(&owner).unwrap(), raw);
        }
    }

    #[test]
    fn unknown_rendition_falls_to_other_and_roundtrips() {
        let rendition: Rendition = serde_json::from_str("\"hdr\"").unwrap();
        assert_eq!(rendition, Rendition::Other("hdr".into()));
        assert_eq!(rendition.as_str(), "hdr");
        assert_eq!(serde_json::to_value(&rendition).unwrap(), "hdr");
        assert_eq!(
            serde_json::to_value(Rendition::Playback).unwrap(),
            "playback"
        );
    }

    #[test]
    fn actions_are_tagged_and_unknown_ones_are_rejected() {
        let action = LibraryAction::CreateAlbum {
            title: "家庭群本周".into(),
            ids: vec!["a1".into()],
            share_with: vec!["mom".into()],
        };
        let json = serde_json::to_value(&action).unwrap();
        assert_eq!(json["kind"], "create_album");
        assert_eq!(roundtrip(&action), action);

        let bare: LibraryAction =
            serde_json::from_value(serde_json::json!({"kind": "create_album", "title": "空"}))
                .unwrap();
        assert!(matches!(bare, LibraryAction::CreateAlbum { ids, .. } if ids.is_empty()));

        let unknown = serde_json::from_value::<LibraryAction>(
            serde_json::json!({"kind": "rotate", "ids": ["a1"]}),
        );
        assert!(unknown.is_err());
    }

    #[test]
    fn each_action_names_the_kind_it_is_sent_as() {
        let ids = || vec!["a1".to_owned()];
        let album = || "al1".to_owned();
        let actions = [
            LibraryAction::Favorite {
                ids: ids(),
                favorite: true,
            },
            LibraryAction::CreateAlbum {
                title: "t".into(),
                ids: ids(),
                share_with: Vec::new(),
            },
            LibraryAction::AddToAlbum {
                album: album(),
                ids: ids(),
            },
            LibraryAction::RemoveFromAlbum {
                album: album(),
                ids: ids(),
            },
            LibraryAction::ShareAlbum {
                album: album(),
                people: vec!["mom".into()],
            },
            LibraryAction::Trash { ids: ids() },
            LibraryAction::Restore { ids: ids() },
        ];
        for action in actions {
            let tag = serde_json::to_value(&action).unwrap()["kind"].clone();
            assert_eq!(serde_json::to_value(action.kind()).unwrap(), tag);
        }
    }

    #[test]
    fn features_keep_unknown_actions_and_default_to_read_only() {
        let features: LibraryFeatures = serde_json::from_value(serde_json::json!({
            "actions": ["favorite", "rotate"],
        }))
        .unwrap();
        assert_eq!(
            features.actions,
            vec![
                LibraryActionKind::Favorite,
                LibraryActionKind::Other("rotate".into())
            ]
        );
        assert_eq!(roundtrip(&features), features);
        let read_only: LibraryFeatures = serde_json::from_value(serde_json::json!({})).unwrap();
        assert!(read_only.actions.is_empty());
    }

    #[test]
    fn action_results_carry_failures_by_item() {
        let result = LibraryActionResult {
            changed: 2,
            album: Some(album()),
            failed: vec![ActionFailure {
                id: "a9".into(),
                failure: MediaFailure {
                    code: MediaFailureCode::PermissionDenied,
                    message: "not the owner".into(),
                    retry_after_secs: None,
                },
            }],
        };
        assert_eq!(roundtrip(&result), result);
        let empty: LibraryActionResult =
            serde_json::from_value(serde_json::json!({"changed": 0})).unwrap();
        assert!(empty.failed.is_empty() && empty.album.is_none());
    }

    #[test]
    fn search_query_omits_unset_fields() {
        let query = SearchQuery {
            q: Some("海边日落".into()),
            kind: Some(MediaKind::Photo),
            ..SearchQuery::default()
        };
        let json = serde_json::to_value(&query).unwrap();
        assert_eq!(json, serde_json::json!({"q": "海边日落", "kind": "photo"}));
        assert_eq!(roundtrip(&query), query);
        assert_eq!(
            query.path(),
            "/library/search?q=%E6%B5%B7%E8%BE%B9%E6%97%A5%E8%90%BD&kind=photo"
        );
        assert_eq!(SearchQuery::default().path(), PATH_LIBRARY_SEARCH);
    }

    #[test]
    fn detail_and_people_roundtrip() {
        let detail = PhotoDetail {
            asset: asset(),
            width: Some(4032),
            height: Some(3024),
            file_name: Some("IMG_0001.HEIC".into()),
            size: Some(2_400_000),
            description: None,
            camera: Some("iPhone 15".into()),
            lens: None,
            place: vec!["青岛".into(), "山东".into(), "中国".into()],
            location: Some(GeoPoint {
                lat: 36.06,
                lon: 120.38,
            }),
            people: vec![PersonRef {
                id: "p1".into(),
                name: "奶奶".into(),
            }],
        };
        assert_eq!(roundtrip(&detail), detail);
        let people = People {
            people: vec![PersonEntry {
                id: "p1".into(),
                name: "奶奶".into(),
            }],
        };
        assert_eq!(roundtrip(&people), people);
        let timeline = Timeline {
            buckets: vec![TimelineBucket {
                month: "2026-11".into(),
                count: 120,
            }],
        };
        assert_eq!(roundtrip(&timeline), timeline);
    }
}
