//! 我的应用的一行：插件行、页面行和行上的红点。

use wonton_protocol::{
    InstalledPlugin, MyAppRef, OpenPage, PlacedTab, PluginIcon, PluginState, PluginsViewModel,
    SidebarOpen, TabIcon, TabLayout, TabRef, TabState,
};

use super::{PagesState, StoredApp, StoredItem, StoredPage};

/// 行拆开后、拼 [`SidebarApp`](wonton_protocol::SidebarApp) 前的中间值，`plugin_row` 和 `page_row` 共用。
pub(crate) struct AppRow {
    pub(super) plugin_id: Option<String>,
    pub(super) tab: Option<TabRef>,
    pub(super) title: String,
    pub(super) subtitle: Option<String>,
    pub(super) icon: Option<PluginIcon>,
    pub(super) open: SidebarOpen,
}

/// 插件行的一行：打开动作按插件算。列表没读到时按存的元数据显示（真被卸载的行在卸载时就拿掉了）；
/// Agent 行没有存的 bot 账号，显示不了，先空着。
pub(crate) fn plugin_row(
    entry: &StoredApp,
    plugin_id: &str,
    plugin: Option<&InstalledPlugin>,
) -> Option<AppRow> {
    let icon = |page: &StoredPage| match &page.icon {
        TabIcon::Plugin { icon } => icon.clone(),
        TabIcon::Monochrome { .. } => None,
    };
    let Some(plugin) = plugin else {
        let page = entry.page.as_ref()?;
        return Some(AppRow {
            plugin_id: Some(plugin_id.to_owned()),
            tab: None,
            title: entry.title.clone().unwrap_or_else(|| page.title.clone()),
            subtitle: None,
            icon: icon(page),
            open: SidebarOpen::Page {
                tab: TabRef::Plugin {
                    plugin_id: plugin_id.to_owned(),
                    page: None,
                },
            },
        });
    };
    let open = if plugin.kind == wonton_protocol::PluginKind::Agent
        && let Some(bot_user_id) = plugin.bot_user_id.clone()
    {
        SidebarOpen::BotChat { bot_user_id }
    } else if let Some(url) = plugin.link_url.clone() {
        SidebarOpen::ExternalBrowser { url }
    } else if plugin.web_url.is_some() {
        SidebarOpen::Page {
            tab: TabRef::Plugin {
                plugin_id: plugin_id.to_owned(),
                page: plugin.tabs.first().map(|page| page.id.clone()),
            },
        }
    } else {
        return None;
    };
    Some(AppRow {
        plugin_id: Some(plugin_id.to_owned()),
        tab: None,
        title: entry.title.clone().unwrap_or_else(|| plugin.title.clone()),
        // 主人改过名的行，原来的用途不一定还对得上，不显示。
        subtitle: entry
            .title
            .is_none()
            .then(|| plugin.subtitle.clone())
            .flatten(),
        icon: plugin.icon.clone(),
        open,
    })
}

/// 页面行的一行：外部网页插件的行（页面行和插件行都可能是它）打开的是系统的 App 内浏览器（ADR-0080）。
pub(crate) fn page_row(
    state: &PagesState,
    entry: &StoredApp,
    tab: &TabRef,
    plugin_id: Option<&str>,
    plugin: Option<&InstalledPlugin>,
) -> AppRow {
    let page = entry
        .page
        .clone()
        .expect("page rows always keep their metadata");
    let _live_problem = state
        .failures
        .contains_key(&PlacedTab::Page { tab: tab.clone() });
    let open = match (tab, plugin) {
        (TabRef::Plugin { .. }, Some(plugin)) if plugin.web_url.is_none() => plugin
            .link_url
            .clone()
            .map(|url| SidebarOpen::ExternalBrowser { url }),
        _ => Some(SidebarOpen::Page { tab: tab.clone() }),
    }
    .expect("page rows of openable plugins always have an open action");
    let icon = match &page.icon {
        TabIcon::Plugin { icon } => icon.clone(),
        TabIcon::Monochrome { .. } => None,
    };
    AppRow {
        plugin_id: plugin_id.map(str::to_owned),
        tab: Some(tab.clone()),
        title: entry.title.clone().unwrap_or(page.title),
        subtitle: None,
        icon,
        open,
    }
}

/// 页面出了问题要不要红点：插件坏了（降级）、或这一行对应的打开页面被新版本移走。
pub(crate) fn problem_badge(
    entry: &StoredApp,
    tab: Option<&TabRef>,
    plugin: Option<&InstalledPlugin>,
    open_pages: &[OpenPage],
) -> bool {
    if plugin.is_some_and(|plugin| matches!(plugin.status, PluginState::Degraded { .. })) {
        return true;
    }
    let matches = |open: &OpenPage| {
        // 行的打开页在插件没页时会退成整个插件入口（page: None），按插件对上。
        let same_plugin = |a: &TabRef, b: &TabRef| match (a, b) {
            (TabRef::Plugin { plugin_id: a, .. }, TabRef::Plugin { plugin_id: b, .. })
            | (TabRef::PluginPath { plugin_id: a, .. }, TabRef::PluginPath { plugin_id: b, .. }) => {
                a == b
            }
            _ => false,
        };
        match tab {
            Some(tab) => &open.tab == tab || same_plugin(&open.tab, tab),
            // 插件行：它任何一个打开着的页面被移走都亮。
            None => match &entry.app {
                MyAppRef::Plugin { plugin_id } => {
                    super::super::tab_plugin_id(&open.tab) == Some(plugin_id.as_str())
                }
                _ => false,
            },
        }
    };
    open_pages
        .iter()
        .any(|open| matches(open) && open.page.state == TabState::PageRemoved)
}

/// 插件列表没到、或插件不再声明这一页时显示用的元数据。栈和我的应用的行存着一份；面板里的条目只存了名字
/// （主面板的在 `layout.json`，房间的在别的客户端也能写的账号数据里），没存的按插件现有的信息补，
/// 列表没到就拿插件 ID 当名字。这里不能 panic：启动和每次开页、关页都要重算视图，算不出来主人就关不掉页面。
pub(crate) fn stored_or_placeholder(
    stored: &StoredItem,
    plugin: Option<&InstalledPlugin>,
    plugins: &PluginsViewModel,
) -> StoredPage {
    stored.page.clone().unwrap_or_else(|| {
        let plugin_title = plugin_title_of(
            super::super::tab_plugin_id(&stored.tab).unwrap_or_default(),
            plugins,
        );
        StoredPage {
            title: plugin_title.clone(),
            plugin_title,
            icon: TabIcon::Plugin {
                icon: plugin.and_then(|plugin| plugin.icon.clone()),
            },
            layout: TabLayout::Inset,
        }
    })
}
pub(crate) fn plugin_title_of(plugin_id: &str, plugins: &PluginsViewModel) -> String {
    plugins
        .installed
        .as_ref()
        .and_then(|list| list.plugins.iter().find(|plugin| plugin.id == plugin_id))
        .map(|plugin| plugin.title.clone())
        .unwrap_or_else(|| plugin_id.to_owned())
}
