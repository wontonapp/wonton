//! 侧边栏（聊天页左上角按钮或左边缘右滑打开）：核心算好交给壳渲染的内容。打开和收起的手势、推开动画、
//! 服务器和连接状态在壳里（[`crate::HomeViewModel`]）。

use serde::{Deserialize, Serialize};

use crate::{PluginIcon, TabRef};

/// 侧边栏的内容，在 [`crate::PluginsViewModel::sidebar`] 里：装好的应用和角标数字都由核心算，壳不自己数。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct SidebarViewModel {
    /// "我的应用"：装好、点了能打开的插件，按已安装列表的顺序，最多 8 个；没列出的只见"全部应用"。
    #[serde(default)]
    pub apps: Vec<SidebarApp>,
    /// 装的应用比 [`SidebarViewModel::apps`] 列出的多：小标题旁的"全部"和最后一行都进应用网格页。
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
    pub plugin_id: String,
    pub title: String,
    /// 点了这一行打开什么。
    pub open: SidebarOpen,
    /// 没有时壳画占位。
    pub icon: Option<PluginIcon>,
    /// 有可升级的版本：行尾一个红点（[`crate::PluginsViewModel::updatable`] 里有它）。
    pub update: bool,
}

/// 点了一行"我的应用"打开什么。长按固定到底栏用的页面清单不在这里：壳从已安装列表的 `tabs` 里拿。
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
                    plugin_id: "app.example.rss".into(),
                    title: "订阅".into(),
                    open: SidebarOpen::Page {
                        tab: TabRef::Plugin {
                            plugin_id: "app.example.rss".into(),
                            page: Some("feed".into()),
                        },
                    },
                    icon: None,
                    update: true,
                },
                SidebarApp {
                    plugin_id: "bot.hermes".into(),
                    title: "Hermes".into(),
                    open: SidebarOpen::BotChat {
                        bot_user_id: "@hermes:home".into(),
                    },
                    icon: None,
                    update: false,
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
