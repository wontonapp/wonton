//! 系统入口（桌面小组件、控制中心按钮、系统搜索）读的快照（ADR-0100）。客户端核心在 App 进程里把它写成 JSON 文件，
//! 入口不跑核心，只读这份文件渲染；点进来走 [`AppLink`](crate::AppLink)。
//!
//! iOS 的小组件扩展不链接客户端核心，自己按字段名解 JSON：字段名由这里的契约测试锁住，改名就是改格式，要升
//! [`WIDGET_SNAPSHOT_VERSION`]。

use serde::{Deserialize, Serialize};

use crate::{BotActivity, RoomPreview, ThemeColors};

/// 快照格式的版本。入口只读自己认识的版本，不认识的按读不了处理。
pub const WIDGET_SNAPSHOT_VERSION: u32 = 1;

/// 快照文件相对 App 数据目录的路径（iOS 的数据目录在 App Group 容器里，扩展读得到）。
pub const WIDGET_SNAPSHOT_PATH: &str = "widget/snapshot.json";

/// 最近会话最多放几个。
pub const WIDGET_MAX_ROOMS: usize = 8;

/// 系统入口的快照。核心写，入口只读、不推导。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct WidgetSnapshot {
    /// 等于写它的核心的 [`WIDGET_SNAPSHOT_VERSION`]。
    pub version: u32,
    /// 写下的时刻，Unix 毫秒。入口凭它显示“上次更新于”。
    pub updated_at_ms: u64,
    pub account: WidgetAccount,
    /// 全部会话的未读数之和（不只是 `rooms` 里的几个）。
    pub unread_total: u64,
    /// 最近会话，按最近活动排，最多 [`WIDGET_MAX_ROOMS`] 个。
    pub rooms: Vec<WidgetRoom>,
    /// 默认 Agent；没选时为空。
    pub default_agent: Option<WidgetAgent>,
    /// 系统搜索的人：有私聊的好友和 Bot，点了进私聊。只有名字，不带消息。
    pub contacts: Vec<WidgetContact>,
    /// 系统搜索的插件：已装、有网页的，点了开插件页。
    pub plugins: Vec<WidgetPlugin>,
    /// App 的浅色、深色颜色令牌（ADR-0026），入口照着上色。
    pub light: ThemeColors,
    pub dark: ThemeColors,
}

/// 快照里的账号状态。不是 [`WidgetAccount::SignedIn`] 时其余列表都是空的。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum WidgetAccount {
    /// 这台手机还没配对家里的服务器（或者配过的都移除了）。
    NotPaired,
    /// 登录失效：设备被吊销，要重新配对或登录。
    SignedOut,
    /// 刚换到另一台服务器，那台的数据还没到：上一台的已经清掉。
    Switching,
    SignedIn,
}

/// 最近会话的一行。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct WidgetRoom {
    /// 点了打开 `wonton://room/<room_id>`。
    pub room_id: String,
    pub name: String,
    pub is_direct: bool,
    pub unread_count: u64,
    /// 最新一条消息的类别和原文，壳按类别写占位文字；还没有消息时为空。
    pub preview: Option<RoomPreview>,
}

/// 默认 Agent。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct WidgetAgent {
    pub user_id: String,
    pub name: String,
    /// 为空表示不确定（没连上家里的服务器，或最近没取到状态）。
    pub activity: Option<BotActivity>,
    /// 和它的私聊的未读数；还没有私聊时为 0。
    pub unread_count: u64,
}

/// 有私聊的一个人或 Bot。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct WidgetContact {
    pub user_id: String,
    pub name: String,
    pub is_bot: bool,
    /// 和他的私聊，点了打开 `wonton://room/<room_id>`。
    pub room_id: String,
}

/// 已装、有网页的插件。插件声明的组件数据（ADR-0100 决策 6 留的口子）也放在这一条里。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct WidgetPlugin {
    /// 点了打开 `wonton://tab/plugin/<plugin_id>`。
    pub plugin_id: String,
    pub title: String,
}

/// 入口读快照文件的结果。
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[expect(
    clippy::large_enum_variant,
    reason = "passed by value across FFI once per read, so variant sizes do not matter; UniFFI fields do not support Box"
)]
pub enum WidgetSnapshotRead {
    /// 核心还没写过：App 装好后还没打开过。
    Missing,
    /// 文件坏了，或者是不认识的版本。`detail` 只进日志。
    Unreadable {
        detail: String,
    },
    Ready {
        snapshot: WidgetSnapshot,
    },
}

impl WidgetSnapshotRead {
    /// 按快照文件的内容判断；`None` 是文件不存在。
    pub fn from_json(json: Option<&[u8]>) -> Self {
        let Some(json) = json else {
            return Self::Missing;
        };
        // 先只看版本：新版本可能改了别的字段，按旧格式整个解析会报出误导的错误。
        #[derive(Deserialize)]
        struct Version {
            version: u32,
        }
        match serde_json::from_slice::<Version>(json) {
            Ok(Version { version }) if version != WIDGET_SNAPSHOT_VERSION => {
                return Self::Unreadable {
                    detail: format!(
                        "widget snapshot version {version}, this build reads {WIDGET_SNAPSHOT_VERSION}"
                    ),
                };
            }
            Ok(_) => {}
            Err(error) => {
                return Self::Unreadable {
                    detail: error.to_string(),
                };
            }
        }
        match serde_json::from_slice(json) {
            Ok(snapshot) => Self::Ready { snapshot },
            Err(error) => Self::Unreadable {
                detail: error.to_string(),
            },
        }
    }
}

#[cfg(test)]
#[path = "widget_tests.rs"]
mod tests;
