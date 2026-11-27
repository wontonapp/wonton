//! 我的应用的配置（ADR-0095）：存进 Matrix 账号数据，登录同一账号的手机都同步；页面实例放在哪一处。

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::{MyAppEntry, TabRef};

/// 全局账号数据事件类型，内容是 [`LayoutSettings`]：我的应用。
pub const LAYOUT_EVENT: &str = "app.wonton.layout";

/// MCP Apps 的宿主页。按 Bot 钉页的 ID 取界面；钉页已去掉，收件箱（ADR-0119）接手宿主后改成按它自己存的页面取。
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

/// 账号数据 [`LAYOUT_EVENT`] 的内容：我的应用。
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
}

impl Default for LayoutSettings {
    fn default() -> Self {
        Self {
            my_apps: Vec::new(),
            auto_add: auto_add_default(),
        }
    }
}

fn auto_add_default() -> bool {
    true
}

/// 一处放着的页面实例：加载、失败、重新加载都按实例记，壳发页面自己的操作时用它指是哪一份。
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "place", rename_all = "snake_case")]
pub enum PlacedTab {
    /// 栈里的那一份。
    Page { tab: TabRef },
    /// 内容窗口的一份；与栈中同一页面共享加载状态，生命周期由订阅持有。
    Content { tab: TabRef },
}

/// 账号数据里不认识的行整条丢掉：旧 App 读新 App 写的配置时，不认识的按钮不显示
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
    }

    #[test]
    fn unknown_entries_are_dropped_not_fatal() {
        let settings: LayoutSettings = serde_json::from_value(serde_json::json!({
            "my_apps": [
                {"app": {"type": "plugin", "plugin_id": "app.example.feed"}},
                {"app": {"type": "hologram", "deck": 3}},
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
        assert!(settings.auto_add);
    }

    #[test]
    fn placed_tab_round_trips_through_json() {
        let placed = PlacedTab::Content {
            tab: TabRef::Plugin {
                plugin_id: "app.example.feed".into(),
                page: Some("feed".into()),
            },
        };
        let json = serde_json::to_value(&placed).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "place": "content",
                "tab": {"type": "plugin", "plugin_id": "app.example.feed", "page": "feed"},
            })
        );
        assert_eq!(serde_json::from_value::<PlacedTab>(json).unwrap(), placed);
    }
}
