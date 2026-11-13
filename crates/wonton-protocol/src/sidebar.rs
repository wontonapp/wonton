//! 侧边栏（聊天页左上角按钮或左边缘右滑打开）：核心算好交给壳渲染的内容。打开和收起的手势、推开动画、
//! 服务器和连接状态在壳里（[`crate::HomeViewModel`]）。
//!
//! "我的应用"由用户选和排序（ADR-0077），存在账号数据里；聊天、商店、设置、新聊天是固定行，不在这份里面。

use serde::{Deserialize, Serialize};

use crate::{PluginIcon, TabRef};

/// 侧边栏的内容，在 [`crate::PluginsViewModel::sidebar`] 里：我的应用和角标数字都由核心算，壳不自己数。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct SidebarViewModel {
    /// "我的应用"：用户选出来的行，按用户排的顺序。
    #[serde(default)]
    pub apps: Vec<SidebarApp>,
    /// 装了还没加进"我的应用"的应用：小标题旁的"全部"和最后一行都进"全部应用"。
    #[serde(default)]
    pub more_apps: bool,
    /// 可更新的应用数：应用商店那一行的角标，和 [`crate::PluginsViewModel::updatable`] 是同一批。
    #[serde(default)]
    pub updates: u32,
    /// 设置里有要处理的事（如核心有新版本）：设置按钮上的红点。
    #[serde(default)]
    pub settings_badge: bool,
}

/// "我的应用"里的一行。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct SidebarApp {
    /// 插件行和插件里的某一页行有它；公网网页行没有。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_id: Option<String>,
    /// 页面行有它（插件页、插件里的某一页、公网网页）；Bot 私聊行没有。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tab: Option<TabRef>,
    pub title: String,
    /// 点了这一行打开什么。
    pub open: SidebarOpen,
    /// 没有时壳画占位。
    #[serde(default)]
    pub icon: Option<PluginIcon>,
    pub badge: SidebarBadge,
}

/// 行尾的红点，两种来源分开画。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct SidebarBadge {
    /// 有可升级的版本（[`crate::PluginsViewModel::updatable`] 里有它）。
    #[serde(default)]
    pub update: bool,
    /// 页面出了问题或插件已停：点了能看原因。
    #[serde(default)]
    pub problem: bool,
    /// 插件已停止，与出错红点分开显示。
    #[serde(default)]
    #[cfg_attr(feature = "uniffi", uniffi(default = false))]
    pub stopped: bool,
}

/// 点了一行"我的应用"打开什么。编辑态的候选不在这里：壳从 [`crate::PagesViewModel`] 的候选里拿。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SidebarOpen {
    /// 和插件的 Bot 的私聊；壳开这个用户的房间。
    BotChat {
        /// Bot 的 Matrix 用户 ID。
        bot_user_id: String,
    },
    /// 插件网页。
    Page { tab: TabRef },
    /// 外部网页插件（`runtime: link`，ADR-0080）：壳用系统的 App 内浏览器打开这个地址，
    /// 顶上显示网址，页面不给任何和 App 通信的桥。
    ExternalBrowser { url: String },
}

#[cfg(test)]
mod tests {
    use serde::Serialize;
    use serde::de::DeserializeOwned;

    use super::*;

    fn round_trip<T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug>(value: T) {
        let json = serde_json::to_value(&value).unwrap();
        assert_eq!(serde_json::from_value::<T>(json).unwrap(), value);
    }

    #[test]
    fn the_view_model_round_trips_through_json() {
        round_trip(SidebarViewModel {
            apps: vec![
                SidebarApp {
                    plugin_id: Some("app.example.rss".into()),
                    tab: Some(TabRef::Plugin {
                        plugin_id: "app.example.rss".into(),
                        page: Some("feed".into()),
                    }),
                    title: "订阅".into(),
                    open: SidebarOpen::Page {
                        tab: TabRef::Plugin {
                            plugin_id: "app.example.rss".into(),
                            page: Some("feed".into()),
                        },
                    },
                    icon: None,
                    badge: SidebarBadge {
                        update: true,
                        problem: false,
                        stopped: false,
                    },
                },
                SidebarApp {
                    plugin_id: Some("bot.hermes".into()),
                    tab: None,
                    title: "Hermes".into(),
                    open: SidebarOpen::BotChat {
                        bot_user_id: "@hermes:home".into(),
                    },
                    icon: None,
                    badge: SidebarBadge::default(),
                },
                SidebarApp {
                    plugin_id: None,
                    tab: Some(TabRef::Url {
                        url: "https://example.com/weather".into(),
                    }),
                    title: "天气".into(),
                    open: SidebarOpen::Page {
                        tab: TabRef::Url {
                            url: "https://example.com/weather".into(),
                        },
                    },
                    icon: None,
                    badge: SidebarBadge::default(),
                },
                SidebarApp {
                    plugin_id: Some("app.example.weather".into()),
                    tab: None,
                    title: "天气".into(),
                    open: SidebarOpen::ExternalBrowser {
                        url: "https://example.com/weather".into(),
                    },
                    icon: None,
                    badge: SidebarBadge::default(),
                },
            ],
            more_apps: true,
            updates: 3,
            settings_badge: true,
        });
    }

    #[test]
    fn an_empty_view_model_round_trips_without_the_fields() {
        let mut value = serde_json::to_value(SidebarViewModel::default()).unwrap();
        value.as_object_mut().unwrap().remove("apps");
        value.as_object_mut().unwrap().remove("more_apps");
        value.as_object_mut().unwrap().remove("updates");
        value.as_object_mut().unwrap().remove("settings_badge");
        assert_eq!(
            serde_json::from_value::<SidebarViewModel>(value).unwrap(),
            SidebarViewModel::default()
        );
    }
}
