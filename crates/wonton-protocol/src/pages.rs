//! 页面（ADR-0095）：引用、状态，和打开页面用的栈。
//!
//! 应用页只有一种打开方式：压进栈、整页铺满（壳按 [`TabPage`] 渲染），关闭弹栈。
//! 我的应用、面板、深链都用 [`TabRef`] 指页面，壳不自己拼。

use serde::{Deserialize, Serialize};

use crate::{PluginIcon, PluginImage};

/// 栈里最多保活几个页面的网页（栈顶往下数）。每多一个多一份整个第三方网页的内存，
/// 和面板的 [`crate::MAX_PANEL_PAGES`] 同一个考虑。
pub const KEEP_ALIVE_PAGES: usize = 3;

/// 插件声明的一个页面（清单 `tabs` 的一项）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PluginTab {
    /// 插件内唯一，插件升级后靠它认出加进过我的应用的页面。
    pub id: String,
    pub title: String,
    /// 单色图标：只有透明通道有意义，颜色由壳上。
    pub icon: PluginImage,
    pub icon_selected: Option<PluginImage>,
    /// `webui` 下的路径，以 `/` 开头。
    pub path: String,
    pub layout: TabLayout,
}

/// 页面躲不躲系统状态栏。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum TabLayout {
    /// 从状态栏下沿开始：现成应用不会被挡。状态栏那一条是 App 的底色。
    #[default]
    Inset,
    /// 铺满屏幕，从状态栏下滚过去，作者用 `--wonton-inset-*` 留出位置。
    EdgeToEdge,
    /// 比这个 App 新的核心给的取值，按 `Inset` 显示。
    #[serde(other)]
    Other,
}

/// 指一个页面。操作都用它，壳从 [`OpenPage::tab`] 或插件列表拿，不自己拼。
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TabRef {
    Chat,
    /// 应用网格页。
    Apps,
    Plugin {
        plugin_id: String,
        /// 清单 `tabs` 里的 `id`；空的是没声明 `tabs` 的插件的网页入口（`webui`）。
        page: Option<String>,
    },
    /// 插件网站里的某一页：网页入口下的路径（含查询和 `#` 后面的部分），只认同一个插件网站。
    /// 从正在看的页面加进我的应用或面板的（[`Action::PinCurrentPage`](crate::Action::PinCurrentPage)），
    /// 或者编辑器里给插件填的路径。
    PluginPath {
        plugin_id: String,
        path: String,
    },
    /// 独立 Bot 网页。选择、我的应用、面板、桥及媒体操作都保留这个身份，不借用插件默认页。
    /// URL 来自同一 BotRef 的 BotView.web；没有已核实的 Ready 时不能用插件 web_url 代替。
    Bot {
        bot: crate::BotRef,
        /// 空值打开清单入口；非空为同一 Bot origin 内的路径，加进我的应用时使用，不存完整 URL。
        #[serde(default)]
        path: Option<String>,
    },
    /// 公网网页，只收 `https://`。手机直接访问，不经隧道：没有登录身份、桥和 `--wonton-*` 变量，
    /// 存储和插件页面分开。
    Url {
        url: String,
    },
    /// 插件的 MCP Apps 界面（`ui://` 资源），Bot 钉上来的（ADR-0052）。核心另给宿主页，
    /// 界面在隔离的 iframe 里，只经 MCP Apps 的 postMessage 说话。
    UiResource {
        plugin_id: String,
        resource_uri: String,
    },
    /// 分享页与本机插件页、其他来源的同名插件分别持有网页状态；不授予主人管理操作。
    SharedApp {
        owner_user_id: String,
        plugin_id: String,
    },
}

/// 主界面和打开着的页面。壳按 [`PagesViewModel::stack`] 渲染，栈顶在最上面。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PagesViewModel {
    /// 不在栈里时露出来的那一页：配对后是聊天，没配对时是应用网格页（配对页也在它上面）。
    pub root: TabRef,
    /// 配对后的应用网格覆盖层；打开应用或聊天时收起，不改变根页面。
    #[serde(default)]
    #[cfg_attr(feature = "uniffi", uniffi(default = false))]
    pub shows_apps: bool,
    /// 打开着的页面，从底到顶；空就是根页面。最后一个元素是当前页。
    pub stack: Vec<OpenPage>,
    /// 刚被卸载、已从我的应用移除的插件的标题，提示一次，见
    /// [`Action::DismissUninstalledTabs`](crate::Action::DismissUninstalledTabs)。
    pub uninstalled: Vec<String>,
    /// 配置文件读不了，我的应用退回默认。改任何一项后，文件重写，这里变空。
    pub problem: Option<PagesProblem>,
    /// 还没加进我的应用、可以加进去的：已装插件的页面，按插件顺序。我的应用本身在
    /// [`crate::SidebarViewModel::apps`]，侧边栏和编辑态渲染同一份。
    #[serde(default)]
    pub my_app_candidates: Vec<TabCandidate>,
    /// 新装的应用自动加到我的应用末尾，见 [`Action::SetAutoAddApps`](crate::Action::SetAutoAddApps)。
    pub auto_add: bool,
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
    /// 本机改动还没同步上账号数据（多半是断网）：按不确定显示，连上后自动重试，不是错误。
    #[serde(default)]
    pub sync_stale: bool,
    /// 每个 Bot 往主面板钉页的同意（服务器上的）；设置里按它显示和收回。
    #[serde(default)]
    pub bot_pin_consents: Vec<crate::BotPinOwner>,
    /// 插件页在问摄像头和麦克风：壳画成平台风格的弹窗（允许 / 不允许）。一次只有一张，
    /// 页面关掉或答完就清；决定按插件记在本机（[`Action::SetPluginMediaPermission`](crate::Action::SetPluginMediaPermission)）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_prompt: Option<TabMediaPrompt>,
}

/// 栈里的一页，壳按它显示胶囊和页面容器。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct OpenPage {
    pub tab: TabRef,
    pub page: TabPage,
}

/// 一个可以加进我的应用、或者放进面板的页面。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct TabCandidate {
    pub tab: TabRef,
    /// 插件页才有。
    pub plugin: Option<CandidatePlugin>,
}

/// 候选里的插件页，按插件显示：列表里每一项都用插件自己的图标，和"全部应用"一样。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct CandidatePlugin {
    pub title: String,
    /// 页面的名字；插件声明了不止一个页面时才有。
    pub page_title: Option<String>,
    /// 没有时壳画占位。
    pub icon: Option<PluginIcon>,
}

/// 一个页面，壳按它显示胶囊、状态页和网页容器。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct TabPage {
    pub title: String,
    pub icon: TabIcon,
    /// 页面的完整地址；插件列表还没读到时为空。
    pub url: Option<String>,
    pub layout: TabLayout,
    pub state: TabState,
    /// 页面要重新加载的计数：变了壳就重新加载（重试、升级完、连上后）。
    pub reload: u64,
    /// 这个页面不在 App 里加载，交给系统浏览器：它要跨源隔离，这个平台的 WebView 给不了（ADR-0033）。
    /// 壳用 `ClientCore::plugin_browser_url` 换浏览器打开的地址。
    #[serde(default)]
    #[cfg_attr(feature = "uniffi", uniffi(default = false))]
    pub opens_in_browser: bool,
}

/// 页面的图标。
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
    /// 骨架加"正在启动…"。
    Starting,
    /// 插件已停止；壳显示"已停止"。
    Stopped,
    /// 已加载的留着，顶部提示。
    Upgrading,
    Uninstalling,
    /// "xx 出了问题"；`reason` 是核心写的原因，详情页也显示。
    Failed {
        reason: String,
    },
    /// 新版本不再提供这个页面。
    PageRemoved,
    /// 页面打不开。`status` 是 HTTP 状态码，没拿到响应时为空；`details` 是核心按 App 语言写的
    /// "详情"原文（出错页默认收起的那段，错误码收在里面），壳有这行就不再自己拼。
    LoadFailed {
        failure: TabLoadFailure,
        status: Option<u16>,
        #[serde(default)]
        details: Option<String>,
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

/// 读不了我的应用配置的原因。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PagesProblem {
    /// 核心写的说明，带上文件路径和原因。
    pub message: String,
}

/// 我的应用里的一行是谁。
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MyAppRef {
    /// 一个插件：Bot 插件开和它的私聊，其余开它的第一个页面。
    Plugin { plugin_id: String },
    /// 插件里的某一页或公网网页，单独加进来的一行。
    Page { tab: TabRef },
}

/// 我的应用里的一条，存账号数据。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct MyAppEntry {
    pub app: MyAppRef,
    /// 用户给这一行改的名字（公网网页行常用），没改时为空。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

/// 插件页在问摄像头和麦克风。发请求的必须是这个页面自己的源，页面里嵌的其他源（如第三方 iframe）直接拒，
/// 不出这张询问。要不要摄像头、要不要麦克风分开给，壳按它写询问的文字。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct TabMediaPrompt {
    pub plugin_id: String,
    /// 插件的显示名；装着列表里没有时是插件 ID。
    pub plugin_title: String,
    pub camera: bool,
    pub microphone: bool,
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
            reload: 2,
            opens_in_browser: true,
        }
    }

    #[test]
    fn the_view_model_round_trips_through_json() {
        let image = |path: &str| PluginImage { path: path.into() };
        let monochrome = TabIcon::Monochrome {
            icon: image("/_wonton/api/images/a.png"),
            selected: Some(image("/_wonton/api/images/b.png")),
        };
        let colored = TabIcon::Plugin {
            icon: Some(PluginIcon {
                small: image("/_wonton/api/images/s.png"),
                large: image("/_wonton/api/images/l.png"),
            }),
        };
        let mut stack = Vec::new();
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
                details: Some("应用的网页返回了错误（HTTP 503）。".into()),
            },
            TabState::Unknown,
        ] {
            stack.push(OpenPage {
                tab: page(),
                page: sample_page(state, monochrome.clone()),
            });
        }
        let panel = vec![crate::PanelItem {
            tab: page(),
            page: sample_page(TabState::Ready, colored),
            attention: true,
            source: crate::PanelSource::User,
            movable: true,
        }];
        round_trip(PagesViewModel {
            root: TabRef::Chat,
            shows_apps: true,
            stack,
            uninstalled: vec!["相册".into()],
            sync_stale: true,
            bot_pin_consents: Vec::new(),
            problem: Some(PagesProblem {
                message: "读不了 layout.json".into(),
            }),
            my_app_candidates: vec![TabCandidate {
                tab: page(),
                plugin: Some(CandidatePlugin {
                    title: "RSS".into(),
                    page_title: Some("订阅".into()),
                    icon: None,
                }),
            }],
            auto_add: false,
            media_prompt: Some(TabMediaPrompt {
                plugin_id: "app.example.feed".into(),
                plugin_title: "动态".into(),
                camera: true,
                microphone: true,
            }),
            panel,
            panel_candidates: vec![],
            panel_selected: Some(page()),
        });
    }

    #[test]
    fn page_actions_round_trip_through_json() {
        for action in [
            Action::OpenPage { tab: page() },
            Action::ClosePage,
            Action::AddMyApp {
                app: MyAppRef::Plugin {
                    plugin_id: "app.example.feed".into(),
                },
                title: None,
                index: Some(0),
            },
            Action::RemoveMyApp {
                app: MyAppRef::Page {
                    tab: TabRef::Url {
                        url: "https://example.com/weather".into(),
                    },
                },
            },
            Action::MoveMyApp {
                app: MyAppRef::Plugin {
                    plugin_id: "app.example.feed".into(),
                },
                index: 2,
            },
            Action::SetAutoAddApps { enabled: false },
            Action::ReloadTab {
                page: crate::PlacedTab::Page { tab: page() },
            },
            Action::TabLoadFailed {
                page: crate::PlacedTab::Panel {
                    scope: crate::PanelScope::Main,
                    tab: page(),
                },
                status: Some(503),
            },
            Action::TabLoadSucceeded {
                page: crate::PlacedTab::Page { tab: page() },
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
            },
            Action::SetRoomEntryBlocked {
                room_id: "!room:home.example".into(),
                blocked: true,
            },
        ] {
            round_trip(action);
        }
    }

    #[test]
    fn layout_has_no_place_for_a_display_mode() {
        let icon = serde_json::json!({"path": "/_wonton/api/images/i.png"});
        let tab: PluginTab = serde_json::from_value(
            serde_json::json!({"id": "feed", "title": "动态", "icon": icon, "path": "/feed", "layout": "inset"}),
        )
        .unwrap();
        assert_eq!(tab.layout, TabLayout::Inset);
        let modern: PluginTab = serde_json::from_value(serde_json::json!({
            "id": "feed", "title": "动态", "icon": icon, "path": "/feed", "layout": "edge_to_edge"
        }))
        .unwrap();
        assert_eq!(modern.layout, TabLayout::EdgeToEdge);
        let unknown: PluginTab = serde_json::from_value(serde_json::json!({
            "id": "feed", "title": "动态", "icon": icon, "path": "/feed", "layout": "float"
        }))
        .unwrap();
        assert_eq!(unknown.layout, TabLayout::Other);
        // 带栏时代的 `display` 不再是字段：写进来也不影响读取。
        let legacy: PluginTab = serde_json::from_value(serde_json::json!({
            "id": "feed", "title": "动态", "icon": icon, "path": "/feed", "layout": "edge_to_edge", "display": "fullscreen"
        }))
        .unwrap();
        assert_eq!(legacy.layout, TabLayout::EdgeToEdge);
    }
}
