//! 底栏（ADR-0033）：插件声明的页面，和客户端核心算好、交给壳渲染的底栏。

use serde::{Deserialize, Serialize};

use crate::{PluginIcon, PluginImage};

/// 插件声明的一个底栏页面（清单 `tabs` 的一项）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PluginTab {
    /// 插件内唯一，插件升级后靠它认出固定过的页面。
    pub id: String,
    pub title: String,
    /// 单色图标：只有透明通道有意义，颜色由壳上。
    pub icon: PluginImage,
    /// 选中时的图标；没有时选中只靠底色和字色。
    pub icon_selected: Option<PluginImage>,
    /// `webui` 下的路径，以 `/` 开头。
    pub path: String,
    pub layout: TabLayout,
    pub display: TabDisplay,
}

/// 页面怎么和 App 的栏摆在一起。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum TabLayout {
    /// 夹在两条栏中间，现成应用不会被挡。
    #[default]
    Inset,
    /// 铺满，从栏下面滚过去，作者用 `--wonton-inset-*` 留出位置。
    EdgeToEdge,
    /// 比这个 App 新的核心给的取值，按 `Inset` 显示。
    #[serde(other)]
    Other,
}

/// 作者声明的打开方式。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum TabDisplay {
    #[default]
    Standard,
    Fullscreen,
    /// 比这个 App 新的核心给的取值，按 `Standard` 打开。
    #[serde(other)]
    Other,
}

/// 用户给一个页面选的打开方式，优先于作者声明的 [`TabDisplay`]。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum TabDisplayChoice {
    /// 跟清单。
    #[default]
    Default,
    Standard,
    Fullscreen,
}

/// 指一个页面。操作都用它，壳从 [`TabItem::tab`] 或插件列表拿，不自己拼。
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TabRef {
    Chat,
    Apps,
    Settings,
    Plugin {
        plugin_id: String,
        /// 清单 `tabs` 里的 `id`；空的是没声明 `tabs` 的插件的网页入口（`webui`）。
        page: Option<String>,
    },
    /// 插件网站里的某一页：网页入口下的路径（含查询和 `#` 后面的部分），只认同一个插件网站。
    /// 从正在看的页面钉上来的（[`Action::PinCurrentPage`](crate::Action::PinCurrentPage)），
    /// 或者编辑面板里给插件填的路径。
    PluginPath {
        plugin_id: String,
        path: String,
    },
    /// 公网网页，只收 `https://`。手机直接访问，不经隧道：没有登录身份、桥和 `--wonton-*` 变量，
    /// 存储和插件页面分开。
    Url {
        url: String,
    },
    /// 分享页与本机插件页、其他来源的同名插件分别持有网页状态；不授予主人管理操作。
    SharedApp {
        owner_user_id: String,
        plugin_id: String,
    },
}

/// 整个底栏，以及盖在主界面上的插件页。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct TabsViewModel {
    /// 底栏上的项，按显示顺序。
    pub items: Vec<TabItem>,
    /// 要不要显示整条底栏：栏上除了"聊天"还有别的项才显示，只有"聊天"时聊天页铺到底。核心算，壳不自己数。
    pub show_bar: bool,
    pub selected: TabRef,
    /// 没固定的插件页打开时盖在主界面上：有顶栏，没有底栏。
    pub overlay: Option<TabItem>,
    /// 栏满了：再固定要主人选换掉哪一项，见 [`Action::PinTab`](crate::Action::PinTab)。
    pub full: bool,
    /// 刚被卸载、已从底栏移除的插件的标题，提示一次，见
    /// [`Action::DismissUninstalledTabs`](crate::Action::DismissUninstalledTabs)。
    pub uninstalled: Vec<String>,
    /// 配置文件读不了，底栏退回默认三项。改任何一项后，文件重写，这里变空。
    pub problem: Option<TabsProblem>,
    /// 底栏上显示文字；关掉时只显示图标，见 [`Action::SetTabLabels`](crate::Action::SetTabLabels)。
    pub labels: bool,
    /// 还没放到底栏上、可以加上去的页面：设置在前，然后按已装插件的顺序。
    pub candidates: Vec<TabCandidate>,
    /// 主聊天页下拉面板里的页面，按显示顺序（ADR-0052）。空的时候壳画大加号。
    #[serde(default)]
    pub panel: Vec<crate::PanelItem>,
    /// 还没放进主面板、可以放进去的页面：已装插件的页面，去掉已经在面板里的。
    /// 插件里的某一页和公网网页不走这里：钉当前页（[`Action::PinCurrentPage`](crate::Action::PinCurrentPage)）
    /// 或在编辑面板里填。
    #[serde(default)]
    pub panel_candidates: Vec<TabCandidate>,
    /// 主面板上次看的那一页（本机记住，不同步），下次拉开还是它。没记住时为 `None`，壳从第一页开。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub panel_selected: Option<TabRef>,
}

/// 底栏上的一项。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct TabItem {
    pub tab: TabRef,
    /// 能从栏上拿掉；聊天不能。
    pub removable: bool,
    /// 图标上要不要画角标：插件页出了问题，或者设置里有要处理的事（核心有新版本）。
    pub badge: bool,
    /// 插件页才有；聊天、应用和设置的标题和图标在壳里。
    pub page: Option<TabPage>,
}

/// 一个可以加到底栏上的页面。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct TabCandidate {
    pub tab: TabRef,
    /// 插件页才有；设置的标题和图标在壳里。
    pub plugin: Option<CandidatePlugin>,
}

/// 可以加到底栏上的插件页，按插件显示：列表里每一项都用插件自己的图标，和“应用”页一样。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct CandidatePlugin {
    pub title: String,
    /// 页面的名字；插件声明了不止一个页面时才有。
    pub page_title: Option<String>,
    /// 没有时壳画占位。
    pub icon: Option<PluginIcon>,
}

/// 一个插件页，壳按它显示栏上的图标、顶栏和页面容器。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct TabPage {
    pub title: String,
    pub icon: TabIcon,
    /// 页面的完整地址；插件列表还没读到时为空。
    pub url: Option<String>,
    pub layout: TabLayout,
    pub state: TabState,
    /// 停止时底栏图标变淡。
    #[serde(default)]
    pub dimmed: bool,
    pub display: TabDisplayChoice,
    /// 算上用户的选择和清单，这个页面打开就是全屏。“总是全屏”的开关显示它。
    pub opens_fullscreen: bool,
    /// 这个页面此刻是全屏：选中时隐藏 App 的栏，只留胶囊。
    pub fullscreen: bool,
    /// 页面要重新加载的计数：变了壳就重新加载（重试、升级完、连上后）。
    pub reload: u64,
}

/// 底栏图标的两种画法。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TabIcon {
    /// 清单 `tabs` 的单色图标，壳上色。
    Monochrome {
        icon: PluginImage,
        selected: Option<PluginImage>,
    },
    /// 没声明 `tabs` 的插件用自己的彩色图标，圆角方形显示，不上色；没有图标时壳画占位。
    Plugin { icon: Option<PluginIcon> },
}

/// 页面容器显示什么。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum TabState {
    /// 插件列表还没读到：骨架。
    Loading,
    /// 加载页面。
    Ready,
    /// 骨架加“正在启动…”。
    Starting,
    /// 插件已停止；壳显示“已停止”，底栏图标变淡。
    Stopped,
    /// 已加载的留着，顶部提示。
    Upgrading,
    Uninstalling,
    /// “xx 出了问题”；`reason` 是核心写的原因，详情页也显示。
    Failed {
        reason: String,
    },
    /// 新版本不再提供这个页面。
    PageRemoved,
    /// 页面打不开。`status` 是 HTTP 状态码，没拿到响应时为空。
    LoadFailed {
        failure: TabLoadFailure,
        status: Option<u16>,
    },
    /// 核心给的插件状态这个 App 不认识，不知道页面能不能用。
    #[serde(other)]
    Unknown,
}

/// 页面打不开的原因。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum TabLoadFailure {
    /// 连不上家里的服务器。
    Transport,
    /// 插件还没就绪。
    PluginNotReady,
    /// 插件的网页出错。
    Upstream,
}

/// 读不了底栏配置的原因。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct TabsProblem {
    /// 核心写的说明，带上文件路径和原因。
    pub message: String,
}

#[cfg(test)]
mod tests {
    use serde::Serialize;
    use serde::de::DeserializeOwned;

    use super::*;
    use crate::Action;

    fn page() -> TabRef {
        TabRef::Plugin {
            plugin_id: "app.example.feed".into(),
            page: Some("feed".into()),
        }
    }

    fn round_trip<T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug>(value: T) {
        let json = serde_json::to_value(&value).unwrap();
        assert_eq!(serde_json::from_value::<T>(json).unwrap(), value);
    }

    fn sample_page(state: TabState, icon: TabIcon) -> TabPage {
        TabPage {
            title: "动态".into(),
            icon,
            url: Some("http://app-example-feed.wonton.localhost:8480/feed".into()),
            layout: TabLayout::EdgeToEdge,
            state,
            dimmed: true,
            display: TabDisplayChoice::Fullscreen,
            opens_fullscreen: true,
            fullscreen: true,
            reload: 2,
        }
    }

    fn sample_item(state: TabState, icon: TabIcon) -> TabItem {
        TabItem {
            tab: page(),
            removable: true,
            badge: true,
            page: Some(sample_page(state, icon)),
        }
    }

    #[test]
    fn the_view_model_round_trips_through_json() {
        let image = |path: &str| PluginImage { path: path.into() };
        let monochrome = TabIcon::Monochrome {
            icon: image("/_wonton/api/images/a.png"),
            selected: Some(image("/_wonton/api/images/b.png")),
        };
        let mut without_dimmed =
            serde_json::to_value(sample_page(TabState::Ready, monochrome.clone())).unwrap();
        without_dimmed.as_object_mut().unwrap().remove("dimmed");
        assert!(
            !serde_json::from_value::<TabPage>(without_dimmed)
                .unwrap()
                .dimmed
        );
        let colored = TabIcon::Plugin {
            icon: Some(PluginIcon {
                small: image("/_wonton/api/images/s.png"),
                large: image("/_wonton/api/images/l.png"),
            }),
        };
        let mut items = vec![TabItem {
            tab: TabRef::Chat,
            removable: false,
            badge: false,
            page: None,
        }];
        for state in [
            TabState::Loading,
            TabState::Ready,
            TabState::Starting,
            TabState::Stopped,
            TabState::Upgrading,
            TabState::Uninstalling,
            TabState::Failed {
                reason: "容器停了".into(),
            },
            TabState::PageRemoved,
            TabState::LoadFailed {
                failure: TabLoadFailure::Upstream,
                status: Some(503),
            },
            TabState::Unknown,
        ] {
            items.push(sample_item(state, monochrome.clone()));
        }
        let overlay = sample_item(TabState::Ready, colored.clone());
        let panel = vec![crate::PanelItem {
            tab: page(),
            page: sample_page(TabState::Ready, colored),
            attention: true,
            source: crate::PanelSource::User,
        }];
        round_trip(TabsViewModel {
            items,
            show_bar: true,
            selected: page(),
            overlay: Some(TabItem {
                tab: TabRef::Plugin {
                    plugin_id: "app.example.other".into(),
                    page: None,
                },
                removable: true,
                badge: false,
                page: Some(overlay.page.expect("overlay page")),
            }),
            full: true,
            uninstalled: vec!["相册".into()],
            problem: Some(TabsProblem {
                message: "读不了 tabs.json".into(),
            }),
            labels: false,
            candidates: vec![
                TabCandidate {
                    tab: TabRef::Settings,
                    plugin: None,
                },
                TabCandidate {
                    tab: page(),
                    plugin: Some(CandidatePlugin {
                        title: "RSS".into(),
                        page_title: Some("订阅".into()),
                        icon: None,
                    }),
                },
            ],
            panel,
            panel_candidates: vec![],
            panel_selected: Some(page()),
        });
    }

    #[test]
    fn tab_actions_round_trip_through_json() {
        for action in [
            Action::SelectTab { tab: page() },
            Action::CloseTabOverlay,
            Action::PinTab {
                tab: page(),
                replacing: Some(TabRef::Settings),
            },
            Action::UnpinTab {
                tab: TabRef::Settings,
            },
            Action::MoveTab {
                tab: TabRef::Apps,
                index: 2,
            },
            Action::SetTabLabels { visible: false },
            Action::SetTabDisplay {
                tab: page(),
                display: TabDisplayChoice::Standard,
            },
            Action::EnterFullscreen { tab: page() },
            Action::ExitFullscreen { tab: page() },
            Action::ReloadTab {
                page: crate::PlacedTab::Bar { tab: page() },
            },
            Action::TabLoadFailed {
                page: crate::PlacedTab::Panel {
                    scope: crate::PanelScope::Main,
                    tab: page(),
                },
                status: Some(503),
            },
            Action::TabLoadSucceeded {
                page: crate::PlacedTab::Bar { tab: page() },
            },
            Action::DismissUninstalledTabs,
            Action::AddPanelPage {
                scope: crate::PanelScope::Main,
                tab: crate::TabRef::Url {
                    url: "https://example.com/weather".into(),
                },
            },
            Action::RemovePanelPage {
                scope: crate::PanelScope::Room {
                    room_id: "!room:home.example".into(),
                },
                tab: page(),
            },
            Action::MovePanelPage {
                scope: crate::PanelScope::Main,
                tab: page(),
                index: 1,
            },
            Action::SelectPanelPage {
                scope: crate::PanelScope::Main,
                tab: page(),
            },
            Action::PinCurrentPage {
                url: "https://app.example.wonton.localhost/stats".into(),
                title: Some("本月统计".into()),
                to: crate::PinTarget::Panel {
                    scope: crate::PanelScope::Main,
                },
                replacing: None,
            },
            Action::SetRoomEntryBlocked {
                room_id: "!room:home.example".into(),
                blocked: true,
            },
        ] {
            round_trip(action);
        }
    }
}
