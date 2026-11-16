//! 聊天页的下拉面板（ADR-0052）：配置存进 Matrix 账号数据，壳按 ViewModel 渲染、发操作。
//!
//! 面板挂在两处：主聊天页（会话列表到顶后往下拉）和每个房间（消息列表到底后往上推）。
//! 用户放的页面存在账号数据里（全局 [`LAYOUT_EVENT`]，房间 [`ROOM_PANEL_EVENT`]），登录同一账号的手机都同步；
//! 上次停在哪一页是本机状态，不同步。Bot 钉页面走 MCP 总线的工具（另见 #35），不写账号数据。

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::{MyAppEntry, TabRef};

/// 全局账号数据事件类型，内容是 [`LayoutSettings`]：我的应用和主面板。
pub const LAYOUT_EVENT: &str = "app.wonton.layout";

/// 房间账号数据事件类型，内容是 [`RoomPanel`]：这个房间的面板。
pub const ROOM_PANEL_EVENT: &str = "app.wonton.room_panel";

/// 面板最多放几页。每多一页多一份 WebView 的内存，和 [`crate::KEEP_ALIVE_PAGES`] 同一个考虑。
pub const MAX_PANEL_PAGES: usize = 5;

/// Bot 钉的页面的列表，以及收回同意。
pub const BOT_PANEL_PINS_PATH: &str = "/_wonton/api/panel/pins";

/// 移除一个 Bot 钉的页面。
pub fn bot_panel_pin_path(id: &str) -> String {
    format!("{BOT_PANEL_PINS_PATH}/{id}")
}

/// 用户看到了这一页，清掉红点。
pub fn bot_panel_pin_seen_path(id: &str) -> String {
    format!("{BOT_PANEL_PINS_PATH}/{id}/seen")
}

/// 一个 Bot 往主面板钉页的同意。
pub fn bot_panel_consent_path(plugin_id: &str, bot: &str) -> String {
    format!("{BOT_PANEL_PINS_PATH}/consent/{plugin_id}/{bot}")
}

/// MCP Apps 的宿主页：一个钉页的界面。
pub fn mcp_app_host_path(pin_id: &str) -> String {
    format!("/_wonton/api/mcp-apps/host/{pin_id}")
}

/// 从 MCP Apps 界面发起的工具调用：新建和查结果。
pub const MCP_APP_CALLS_PATH: &str = "/_wonton/api/mcp-apps/calls";
pub fn mcp_app_call_path(id: &str) -> String {
    format!("{MCP_APP_CALLS_PATH}/{id}")
}

/// MCP Apps 界面读同一个插件的资源。
pub const MCP_APP_READ_PATH: &str = "/_wonton/api/mcp-apps/read";

/// 界面能调的工具列表（只列声明给界面用的）。
pub const MCP_APP_TOOLS_PATH: &str = "/_wonton/api/mcp-apps/tools";

/// 界面以主人的名义发一条消息。
pub const MCP_APP_MESSAGE_PATH: &str = "/_wonton/api/mcp-apps/message";

/// 面板在哪个面上：主聊天页，或某个房间。每个房间的面板单独设，互不影响。
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PanelScope {
    /// 主聊天页的面板。
    Main,
    /// 这个房间的面板，`room_id` 是 Matrix 房间 ID。
    Room { room_id: String },
}

/// 面板里用户放的一项：指向哪个页面，和用户起的名字。
///
/// 同一个页面可以同时在栈里和面板里，两处是两个实例，各自加载。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PanelEntry {
    pub tab: TabRef,
    /// 用户起的名字；没有时核心按页面补（插件页用清单标题，网址用域名）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

/// 账号数据 [`LAYOUT_EVENT`] 的内容：我的应用和主面板。
///
/// 客户端改一项时按原始 JSON 只改那一处，其余原样写回：旧 App 读到不认识的取值会丢掉那一项，
/// 整份重写会把别的设备存的配置冲掉（写法同 [`crate::ChatLayoutSettings`]）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayoutSettings {
    /// 我的应用里的行，按用户排的顺序。
    #[serde(default, deserialize_with = "drop_unknown")]
    pub my_apps: Vec<MyAppEntry>,
    /// 新装的应用自动加到我的应用末尾。缺这一项时按开算。
    #[serde(default = "auto_add_default")]
    pub auto_add: bool,
    /// 主面板里的页面，按显示顺序。
    #[serde(default, deserialize_with = "drop_unknown")]
    pub panel: Vec<PanelEntry>,
}

impl Default for LayoutSettings {
    fn default() -> Self {
        Self {
            my_apps: Vec::new(),
            auto_add: auto_add_default(),
            panel: Vec::new(),
        }
    }
}

fn auto_add_default() -> bool {
    true
}

/// 房间账号数据 [`ROOM_PANEL_EVENT`] 的内容：这个房间的面板。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomPanel {
    /// 面板里的页面，按显示顺序。
    #[serde(default, deserialize_with = "drop_unknown")]
    pub pages: Vec<PanelEntry>,
}

/// 一处放着的页面实例。同一个页面可以同时在栈里和面板里，两边互不影响：
/// 加载、失败、重新加载都按实例记（ADR-0052），壳发页面自己的操作时用它指是哪一份。
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "place", rename_all = "snake_case")]
pub enum PlacedTab {
    /// 栈里的那一份。
    Page { tab: TabRef },
    /// 内容窗口的一份；与栈中同一页面共享加载状态，生命周期由订阅持有。
    Content { tab: TabRef },
    /// 面板里的那一份。
    Panel { scope: PanelScope, tab: TabRef },
}

/// 钉到哪里。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PinTarget {
    /// 面板：主面板或某个房间的面板。
    Panel { scope: PanelScope },
    /// 我的应用。
    MyApps,
}

/// 面板里一项的来源。用户随时可以拿掉任何来源的页面。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum PanelSource {
    /// 用户自己放的。
    #[default]
    User,
    /// Bot 钉上来的，壳标明“由 <名字> 钉上”。
    Bot {
        /// 钉页的标识，移除、标已看时用。
        pin_id: String,
        plugin_id: String,
        bot: String,
        /// 显示用的名字（Bot 的显示名）。
        name: String,
    },
    /// 比这个 App 新的核心给的取值，按用户放的显示。
    #[serde(other)]
    Other,
}

/// 一个 Bot 钉的页面（服务器核心存着，App 读出来和用户放的合在一起显示）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct BotPanelPin {
    pub id: String,
    pub scope: PanelScope,
    pub tab: TabRef,
    pub title: String,
    pub attention: bool,
    /// 钉它的插件和 Bot，及 Bot 的显示名。
    pub plugin_id: String,
    pub bot: String,
    pub name: String,
}

/// 全部 Bot 钉的页面。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct BotPanelPins {
    pub pins: Vec<BotPanelPin>,
    /// 每个钉过页或有同意的 Bot，设置里按它显示和收回。
    pub consents: Vec<BotPinOwner>,
}

/// 一个 Bot 往主面板钉页的同意。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct BotPinOwner {
    pub plugin_id: String,
    pub bot: String,
    pub name: String,
    pub main_consent: bool,
}

/// 收回或给回一个 Bot 往主面板钉页的同意。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct BotPinConsent {
    pub allowed: bool,
}

/// 面板里的一项，壳按它画页面容器和圆点。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PanelItem {
    pub tab: TabRef,
    /// 页面的显示数据；面板里都是插件页、插件里的某一页或公网网页，总有这一项。
    pub page: crate::TabPage,
    /// 这页要求关注（红点），还没有被看到过。页面经桥设，用户翻到就清掉。
    pub attention: bool,
    /// 钉上来的来源；Bot 钉的壳要标明。
    pub source: PanelSource,
    /// 能不能在面板里拖动排序。Bot 钉的按钉的先后固定，不能拖。
    #[serde(default = "movable_default")]
    pub movable: bool,
}

fn movable_default() -> bool {
    true
}

/// 账号数据里不认识的页面整条丢掉：旧 App 读新 App 写的配置时，不认识的按钮不显示
/// （AGENTS.md 的兼容规矩），不因为一条不认识就丢掉整份配置。
fn drop_unknown<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: DeserializeOwned,
{
    let values = Vec::<serde_json::Value>::deserialize(deserializer)?;
    Ok(values
        .into_iter()
        .filter_map(|value| serde_json::from_value(value).ok())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MyAppRef;

    fn plugin_page() -> TabRef {
        TabRef::Plugin {
            plugin_id: "app.example.feed".into(),
            page: Some("feed".into()),
        }
    }

    #[test]
    fn layout_settings_round_trips_through_json() {
        let settings = LayoutSettings {
            my_apps: vec![
                MyAppEntry {
                    app: MyAppRef::Plugin {
                        plugin_id: "app.example.feed".into(),
                    },
                    title: None,
                },
                MyAppEntry {
                    app: MyAppRef::Page {
                        tab: TabRef::Url {
                            url: "https://example.com/weather".into(),
                        },
                    },
                    title: Some("天气".into()),
                },
            ],
            auto_add: false,
            panel: vec![PanelEntry {
                tab: plugin_page(),
                title: Some("动态".into()),
            }],
        };
        let json = serde_json::to_value(&settings).unwrap();
        assert_eq!(json["auto_add"], serde_json::json!(false));
        assert_eq!(
            json["my_apps"][1],
            serde_json::json!({
                "app": {"type": "page", "tab": {"type": "url", "url": "https://example.com/weather"}},
                "title": "天气",
            })
        );
        assert_eq!(
            json["panel"][0],
            serde_json::json!({
                "tab": {"type": "plugin", "plugin_id": "app.example.feed", "page": "feed"},
                "title": "动态",
            })
        );
        assert_eq!(
            serde_json::from_value::<LayoutSettings>(json).unwrap(),
            settings
        );
    }

    #[test]
    fn missing_fields_fall_back_to_defaults() {
        let settings: LayoutSettings = serde_json::from_value(serde_json::json!({})).unwrap();
        assert_eq!(settings, LayoutSettings::default());
        assert!(settings.auto_add);
        assert!(settings.my_apps.is_empty());
        assert!(settings.panel.is_empty());

        let panel: RoomPanel = serde_json::from_value(serde_json::json!({})).unwrap();
        assert_eq!(panel, RoomPanel::default());
    }

    #[test]
    fn unknown_entries_are_dropped_not_fatal() {
        let settings: LayoutSettings = serde_json::from_value(serde_json::json!({
            "my_apps": [
                {"app": {"type": "plugin", "plugin_id": "app.example.feed"}},
                {"app": {"type": "hologram", "deck": 3}},
            ],
            "panel": [
                {"tab": {"type": "url", "url": "https://example.com/"}},
                {"tab": {"type": "plugin", "plugin_id": "app.example.feed"}, "title": 7},
            ],
        }))
        .unwrap();
        assert_eq!(
            settings.my_apps,
            vec![MyAppEntry {
                app: MyAppRef::Plugin {
                    plugin_id: "app.example.feed".into(),
                },
                title: None,
            }]
        );
        // 网址页是认识的取值，留下；标题类型不对的那条丢掉。
        assert_eq!(
            settings.panel,
            vec![PanelEntry {
                tab: TabRef::Url {
                    url: "https://example.com/".into(),
                },
                title: None,
            }]
        );
        assert!(settings.auto_add);
    }

    #[test]
    fn unknown_source_falls_back_to_user() {
        let item: PanelSource = serde_json::from_str("{\"source\": \"hydra\"}").unwrap();
        assert_eq!(item, PanelSource::Other);
    }

    #[test]
    fn a_bot_source_carries_the_pin_id_and_falls_back_for_new_values() {
        let source = PanelSource::Bot {
            pin_id: "pin".into(),
            plugin_id: "app.example.agent".into(),
            bot: "hermes".into(),
            name: "Hermes".into(),
        };
        let json = serde_json::to_value(&source).unwrap();
        assert_eq!(json["source"], "bot");
        assert_eq!(json["pin_id"], "pin");
        assert_eq!(serde_json::from_value::<PanelSource>(json).unwrap(), source);
    }

    #[test]
    fn placed_tab_round_trips_through_json() {
        let placed = PlacedTab::Panel {
            scope: PanelScope::Room {
                room_id: "!room:home.example".into(),
            },
            tab: plugin_page(),
        };
        let json = serde_json::to_value(&placed).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "place": "panel",
                "scope": {"type": "room", "room_id": "!room:home.example"},
                "tab": {"type": "plugin", "plugin_id": "app.example.feed", "page": "feed"},
            })
        );
        assert_eq!(serde_json::from_value::<PlacedTab>(json).unwrap(), placed);
    }
}
