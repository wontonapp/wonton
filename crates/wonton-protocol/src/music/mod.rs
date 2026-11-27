//! 原生音乐页（ADR-0113 第 2 节“读路径的例外”）：曲库浏览由客户端核直讲提供方的 OpenSubsonic 接口，
//! 壳按这里的视图画库、专辑、歌手、歌单、搜索和歌词。出声、队列和进度不在这里，走 [`crate::media`] 的播放会话。
//!
//! 一页一个订阅：壳用 [`MusicPage`] 说要哪一页，核心回 [`MusicPageView`]，页内的翻页、重试、搜索词都是
//! [`MusicPageAction`]。收藏这类会改到别的页的动作是 [`MusicAction`]，核心改完把开着的页一起更新。
//!
//! 这些类型只在 App 进程里从核心交给壳，不过网络；核心和壳一起发版，所以枚举不留 `Other`。

use serde::{Deserialize, Serialize};

mod items;

pub use items::*;

/// 一页翻页时每次取多少条。专辑网格一屏放得下二三十张，取两屏。
pub const MUSIC_PAGE_SIZE: u32 = 60;

/// 首页每一排最多几张。
pub const MUSIC_HOME_ROW: u32 = 12;

/// 搜索词停多久才发：边打边搜，打字时不每个字都发。
pub const MUSIC_SEARCH_DEBOUNCE_MS: u64 = 300;

/// 壳要打开的一页。
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "page", rename_all = "snake_case")]
pub enum MusicPage {
    /// 曲库首页：最近添加、最近播放、常听、歌单。
    Home,
    Albums {
        sort: AlbumSort,
    },
    Artists,
    Playlists,
    /// 全部歌曲，按歌名。
    Songs,
    /// 收藏的歌手、专辑和歌。
    Starred,
    Genres,
    /// 一个流派下的专辑。
    Genre {
        name: String,
    },
    Album {
        id: String,
    },
    Artist {
        id: String,
    },
    Playlist {
        id: String,
    },
    /// 搜索页；词由 [`MusicPageAction::Query`] 给，空词显示空页。
    Search,
}

/// 专辑列表的排序（OpenSubsonic `getAlbumList2` 的 `type`）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum AlbumSort {
    /// 最近添加。
    Newest,
    /// 最近播放。
    Recent,
    /// 播放最多。
    Frequent,
    /// 按专辑名。
    Name,
    /// 按歌手名。
    Artist,
    Random,
}

/// 一页的当前样子。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct MusicPageView {
    pub page: MusicPage,
    pub state: MusicPageState,
}

/// 页面容器显示什么。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum MusicPageState {
    /// 第一次取，骨架。
    Loading,
    Ready {
        content: MusicContent,
        /// 还有下一页；壳滚到底发 [`MusicPageAction::LoadMore`]。
        has_more: bool,
        /// 正在取下一页，列表底部转圈。
        loading_more: bool,
        /// 翻页或刷新失败：已有内容留着，底部一条轻提示带重试。
        problem: Option<MusicProblem>,
    },
    /// 一条都没取到：整页出错页。
    Failed { problem: MusicProblem },
}

/// 取一次失败的原因。类别由核心给，文字由壳按类别写；`details` 是收在“详情”里的原文（状态码、上游的话）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct MusicProblem {
    pub failure: MusicFailure,
    pub details: Option<String>,
}

/// 失败的类别（AGENTS.md：传输、权限、插件未就绪、上游、输入分开）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum MusicFailure {
    /// 连不上家里的服务器。
    Transport,
    /// 音乐插件没在跑或还没起来。
    PluginNotReady,
    /// 曲库不认这台设备的人：网关没带出身份，或者建号失败。
    NotSignedIn,
    /// 曲库认得这个人，但不让他做这件事（如改别人的歌单）。
    PermissionDenied,
    /// 要的专辑、歌手、歌单已经不在库里。
    NotFound,
    /// 曲库出错（5xx 或 Subsonic 的通用错误）。
    Upstream,
    /// 曲库回了看不懂的东西。
    Protocol,
}

/// 页内的动作。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum MusicPageAction {
    LoadMore,
    /// 出错后重试，或者下拉刷新：已有内容留着，取到新的再换。
    Reload,
    /// 搜索页的词；核心等 [`MUSIC_SEARCH_DEBOUNCE_MS`] 没有新词才发，旧的一发出就作废。
    Query {
        text: String,
    },
}

/// 改到曲库、可能影响好几页的动作。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum MusicAction {
    /// 加星或取消星：开着的页立刻改，曲库回失败就改回来并把原因交给壳。
    Star { target: MusicTarget, starred: bool },
}

/// 能加星的东西。
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", content = "id", rename_all = "snake_case")]
pub enum MusicTarget {
    Song(String),
    Album(String),
    Artist(String),
}
