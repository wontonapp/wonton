//! 页面栈和我的应用的 ViewModel 映射。

use url::Url;
use wonton_protocol::media::MediaBrowse;
use wonton_protocol::{
    CoreError, InstalledPlugin, MyAppRef, NativePage, OpenPage, PagesViewModel, PlacedTab,
    PluginState, PluginsViewModel, SidebarApp, SidebarBadge, SidebarOpen, TabIcon, TabLayout,
    TabLoadFailure, TabPage, TabRef, TabState,
};

use super::{PagesState, StoredApp, StoredItem, StoredPage, bot, invalid};
use crate::l10n::{CATALOG, language};
use wonton_l10n::l10n;

mod compose;
mod rows;
mod url_page;

pub(crate) use self::compose::{compose_page, my_app_candidates};
pub(crate) use self::rows::{
    AppRow, page_row, plugin_row, plugin_title_of, problem_badge, stored_or_placeholder,
};
pub(crate) use self::url_page::{build_url_page, stored_url_item};

pub(super) fn make_view(
    state: &PagesState,
    plugins: &PluginsViewModel,
    sync_stale: bool,
) -> PagesViewModel {
    let stack = state
        .stack
        .iter()
        .map(|item| OpenPage {
            tab: item.tab.clone(),
            page: build_page_item(state, item, plugins),
        })
        .collect();
    PagesViewModel {
        root: state.root(),
        shows_apps: state.shows_apps,
        stack,
        uninstalled: state.uninstalled.clone(),
        problem: state.problem.clone().or_else(|| state.read_problem.clone()),
        my_app_candidates: my_app_candidates(state, plugins),
        auto_add: state.config.auto_add,
        sync_stale,
        // 询问跟着页面走：页面不在了就不显示（state 里的清理由改动助手做）。
        media_prompt: state
            .media_prompt
            .as_ref()
            .map(|pending| wonton_protocol::TabMediaPrompt {
                plugin_id: pending.plugin_id.clone(),
                plugin_title: plugin_title_of(&pending.plugin_id, plugins),
                camera: pending.camera,
                microphone: pending.microphone,
            }),
    }
}

/// 我的应用的一行。插件行按插件做（Bot 插件开私聊），页面行按缓存的标题和图标做。
pub(super) fn build_app(
    state: &PagesState,
    entry: &StoredApp,
    plugins: &PluginsViewModel,
    open_pages: &[OpenPage],
) -> Option<SidebarApp> {
    let installed = plugins.installed.as_ref();
    let plugin = |plugin_id: &str| {
        installed.and_then(|list| list.plugins.iter().find(|plugin| plugin.id == plugin_id))
    };
    let row = match &entry.app {
        MyAppRef::Plugin { plugin_id } => plugin_row(entry, plugin_id, plugin(plugin_id))?,
        MyAppRef::Page { tab } => page_row(
            state,
            entry,
            tab,
            super::tab_plugin_id(tab),
            super::tab_plugin_id(tab).and_then(plugin),
        ),
    };
    let AppRow {
        plugin_id,
        tab,
        title,
        subtitle,
        icon,
        open,
    } = row;
    let page_tab = tab.or_else(|| match &open {
        SidebarOpen::Page { tab } => Some(tab.clone()),
        SidebarOpen::BotChat { .. } | SidebarOpen::ExternalBrowser { .. } => None,
    });
    let plugin = plugin_id.as_deref().and_then(plugin);
    let problem = problem_badge(entry, page_tab.as_ref(), plugin, open_pages);
    Some(SidebarApp {
        plugin_id: plugin_id.clone(),
        tab: page_tab,
        title,
        subtitle,
        open,
        icon,
        badge: SidebarBadge {
            update: plugin_id.is_some_and(|plugin_id| {
                plugins
                    .updatable
                    .iter()
                    .any(|updatable| updatable.plugin_id == plugin_id)
            }),
            problem,
            stopped: plugin.is_some_and(|plugin| matches!(plugin.status, PluginState::Stopped)),
        },
    })
}

/// 栈里的一页：页面元数据按插件或缓存合成。
pub(super) fn build_page_item(
    state: &PagesState,
    stored: &StoredItem,
    plugins: &PluginsViewModel,
) -> TabPage {
    let placed = PlacedTab::Page {
        tab: stored.tab.clone(),
    };
    if matches!(stored.tab, TabRef::Widget { .. }) {
        return super::widget::page(state, &placed, stored, plugins);
    }
    let plugin = match &stored.tab {
        TabRef::Plugin { plugin_id, .. } | TabRef::PluginPath { plugin_id, .. } => plugins
            .installed
            .as_ref()
            .and_then(|list| list.plugins.iter().find(|plugin| plugin.id == *plugin_id)),
        _ => None,
    };
    let page_id = match &stored.tab {
        TabRef::Plugin { page, .. } => page.as_deref(),
        _ => None,
    };
    build_page(state, &placed, stored, plugin, page_id, plugins)
}

/// 本机插件的页面要跨源隔离、这个平台的 WebView 给不了时交给系统浏览器（ADR-0033）。
pub(super) fn build_page(
    state: &PagesState,
    placed: &PlacedTab,
    stored: &StoredItem,
    plugin: Option<&InstalledPlugin>,
    page_id: Option<&str>,
    plugins: &PluginsViewModel,
) -> TabPage {
    let page = compose_page(state, placed, stored, plugin, page_id, plugins);
    let own_page = matches!(
        stored.tab,
        TabRef::Plugin { .. } | TabRef::PluginPath { .. }
    );
    let browser =
        plugin.is_some_and(|p| crate::browser::opens_in_browser(p, crate::browser::ENGINE));
    TabPage {
        opens_in_browser: own_page && browser,
        native: plugin.and_then(|p| native_page(&stored.tab, p)),
        ..page
    }
}

/// 插件的入口页能换成原生界面时是哪一个：只换入口（`page` 为空的 `TabRef::Plugin`），钉住的子页和“网页版”照旧开网页。
fn native_page(tab: &TabRef, plugin: &InstalledPlugin) -> Option<NativePage> {
    let TabRef::Plugin { page: None, .. } = tab else {
        return None;
    };
    match plugin.media_browse {
        Some(MediaBrowse::OpenSubsonic) => Some(NativePage::Music),
        Some(MediaBrowse::Other(_)) | None => None,
    }
}

/// 插件里的某一页没起名字时，拿路径的末段当标题。
pub(super) fn path_title(path: &str) -> String {
    path.split(['?', '#'])
        .next()
        .unwrap_or(path)
        .rsplit('/')
        .find(|segment| !segment.is_empty())
        .unwrap_or(path)
        .to_owned()
}

pub(crate) fn page_from_metadata(
    state: &PagesState,

    placed: &PlacedTab,
    metadata: StoredPage,
    url: Option<String>,
    state_value: TabState,
    _plugins: &PluginsViewModel,
) -> TabPage {
    TabPage {
        title: metadata.title,
        icon: metadata.icon,
        url,
        layout: normalize_layout(metadata.layout),
        state: state_value,
        reload: state.reloads.get(placed).copied().unwrap_or_default(),
        native: None,
        opens_in_browser: false,
    }
}

pub(super) fn page_state(
    state: &PagesState,
    placed: &PlacedTab,
    plugin: &InstalledPlugin,
    has_url: bool,
) -> TabState {
    if !has_url {
        return TabState::PageRemoved;
    }
    if matches!(plugin.status, PluginState::Stopped) {
        return TabState::Stopped;
    }
    if let Some((failure, status)) = state.failures.get(placed) {
        return TabState::LoadFailed {
            failure: *failure,
            status: *status,
            details: load_failed_details(*failure, *status),
        };
    }
    match &plugin.status {
        PluginState::Installing => TabState::Starting,
        PluginState::Stopped => TabState::Stopped,
        PluginState::Running { .. } => TabState::Ready,
        // 待配置时网页照常，只是插件的工具不能调。
        PluginState::NeedsConfiguration => TabState::Ready,
        PluginState::Upgrading => TabState::Upgrading,
        PluginState::Uninstalling => TabState::Uninstalling,
        PluginState::Degraded { reason } => TabState::Failed {
            reason: reason.clone(),
        },
        PluginState::Unknown => TabState::Unknown,
    }
}

/// 出错页默认收起的"详情"：核心按 App 语言写，HTTP 错误码收在里面（ADR-0032）。没拿到响应
/// 就没有错误码可写，交回空，壳保持只显示错误码的原样。
pub(super) fn load_failed_details(failure: TabLoadFailure, status: Option<u16>) -> Option<String> {
    match (failure, status) {
        (TabLoadFailure::Transport, _) => {
            Some(l10n!(CATALOG, language(), "tabs-load-details-transport"))
        }
        (TabLoadFailure::PluginNotReady, Some(code)) => Some(l10n!(
            CATALOG,
            language(),
            "tabs-load-details-not-ready",
            code = code.to_string()
        )),
        (TabLoadFailure::Upstream, Some(code)) => Some(l10n!(
            CATALOG,
            language(),
            "tabs-load-details-upstream",
            code = code.to_string()
        )),
        (TabLoadFailure::PluginNotReady, None) | (TabLoadFailure::Upstream, None) => None,
    }
}

pub(super) fn metadata_for_plugin(
    plugin: &InstalledPlugin,
    page: Option<&str>,
) -> Option<StoredPage> {
    let Some(page) = page else {
        // 网页入口（`webui`）和外部网页（`link`）都没有声明的页面，行就是整个插件。
        return (plugin.web_url.is_some() || plugin.link_url.is_some()).then(|| StoredPage {
            plugin_title: plugin.title.clone(),
            title: plugin.title.clone(),
            icon: TabIcon::Plugin {
                icon: plugin.icon.clone(),
            },
            layout: TabLayout::Inset,
        });
    };
    let page = plugin.tabs.iter().find(|tab| tab.id == page)?;
    Some(StoredPage {
        plugin_title: plugin.title.clone(),
        title: page.title.clone(),
        icon: TabIcon::Monochrome {
            icon: page.icon.clone(),
            selected: page.icon_selected.clone(),
        },
        layout: normalize_layout(page.layout),
    })
}

/// 公网网页只收 `https://`（ADR-0052）：明文网页和插件页并排出现，容易被当成自家页面。
pub(super) fn is_https_url(url: &str) -> bool {
    Url::parse(url).is_ok_and(|url| url.scheme() == "https" && url.host_str().is_some())
}

/// 插件里的路径必须落在插件自己的网站上，落在外面就按公网网页、别的插件的页算，不是这个插件的。
pub(super) fn checked_plugin_path(
    tab: &TabRef,
    plugin: &InstalledPlugin,
) -> Result<TabRef, CoreError> {
    let TabRef::PluginPath { path, .. } = tab else {
        return Ok(tab.clone());
    };
    if page_url(plugin.web_url.as_deref(), Some(path)).is_some() {
        return Ok(tab.clone());
    }
    Err(invalid(&l10n!(CATALOG, language(), "page-path-outside")))
}

/// 插件网页入口下的网址。路径只留在插件网站内：以 `/` 开头的路径直接接在来源后面，
/// 不能交给 `Url::join`，否则 `//evil.example/x` 会被当成协议相对网址跳到别的主机（ADR-0052）。
pub(super) fn page_url(base: Option<&str>, page: Option<&str>) -> Option<String> {
    let base = Url::parse(base?).ok()?;
    let Some(path) = page else {
        return Some(base.to_string());
    };
    let url = if path.starts_with('/') {
        Url::parse(&format!("{}{path}", base.origin().ascii_serialization())).ok()?
    } else {
        base.join(path).ok()?
    };
    (url.origin() == base.origin()).then(|| url.to_string())
}

/// 钉页面上来的带用户起的名字；候选页来的没有，按插件或域名补。
pub(super) fn stored_item_with_title(
    state: &PagesState,
    tab: &TabRef,
    plugins: &PluginsViewModel,
    title: Option<String>,
) -> Result<StoredItem, CoreError> {
    match tab {
        TabRef::Chat | TabRef::Apps => {
            Err(invalid(&l10n!(CATALOG, language(), "tabs-view-not-a-page")))
        }
        // Bot 网页和分享页的地址还不由核心给（壳拿到的是空地址），标题先按缓存的走。
        TabRef::SharedApp { plugin_id, .. } => Ok(StoredItem {
            tab: tab.clone(),
            page: Some(StoredPage {
                plugin_title: plugin_id.clone(),
                title: plugin_id.clone(),
                icon: TabIcon::Plugin { icon: None },
                layout: TabLayout::Inset,
            }),
        }),
        TabRef::Widget { .. } => Err(invalid(&l10n!(
            CATALOG,
            language(),
            "tabs-widget-not-pinnable"
        ))),
        TabRef::Bot { .. } => bot::stored_item(state, tab, title),
        TabRef::Url { url } => stored_url_item(tab, url, title),
        TabRef::PluginPath { plugin_id, path: _ } => {
            let installed = plugins
                .installed
                .as_ref()
                .ok_or_else(|| CoreError::NotReady {
                    reason: l10n!(CATALOG, language(), "tabs-view-plugin-list-not-ready"),
                })?;
            let plugin = installed
                .plugins
                .iter()
                .find(|plugin| plugin.id == *plugin_id)
                .ok_or_else(|| invalid(&l10n!(CATALOG, language(), "tabs-view-unknown-plugin")))?;
            Ok(StoredItem {
                // 手填的路径必须留在插件网站里（ADR-0052）。
                tab: checked_plugin_path(tab, plugin)?,
                page: Some(StoredPage {
                    plugin_title: plugin.title.clone(),
                    title: title.unwrap_or_else(|| plugin.title.clone()),
                    icon: TabIcon::Plugin {
                        icon: plugin.icon.clone(),
                    },
                    layout: TabLayout::Inset,
                }),
            })
        }
        // MCP Apps 宿主页只随 Bot 钉页出现（核心拉账号数据时建），不从这里手动打开。
        TabRef::UiResource { .. } => {
            Err(invalid(&l10n!(CATALOG, language(), "tabs-view-not-a-page")))
        }
        TabRef::Plugin { plugin_id, page } => {
            let Some(installed) = plugins.installed.as_ref() else {
                return Err(CoreError::NotReady {
                    reason: l10n!(CATALOG, language(), "tabs-view-plugin-list-not-ready"),
                });
            };
            let Some(plugin) = installed
                .plugins
                .iter()
                .find(|plugin| plugin.id == *plugin_id)
            else {
                return Err(invalid(&l10n!(
                    CATALOG,
                    language(),
                    "tabs-view-unknown-plugin"
                )));
            };
            let Some(mut metadata) = metadata_for_plugin(plugin, page.as_deref()) else {
                return Err(invalid(&l10n!(
                    CATALOG,
                    language(),
                    "tabs-view-plugin-no-page"
                )));
            };
            if let Some(title) = title {
                metadata.title = title;
            }
            Ok(StoredItem {
                tab: tab.clone(),
                page: Some(metadata),
            })
        }
    }
}

/// 这个页面现在能不能打开（栈和面板共用）：插件页要清单里还有、插件网页也还在。
pub(super) fn page_available(tab: &TabRef, plugins: &PluginsViewModel) -> bool {
    match tab {
        // MCP Apps 宿主页随 Bot 钉页来：钉页拉取流程建的条目才可打开。
        TabRef::UiResource { .. } => false,
        TabRef::Url { url } => is_https_url(url),
        TabRef::Plugin { plugin_id, page } => plugins
            .installed
            .as_ref()
            .and_then(|list| list.plugins.iter().find(|plugin| plugin.id == *plugin_id))
            .is_some_and(|plugin| match page {
                Some(page) => plugin.tabs.iter().any(|tab| tab.id == *page),
                None => plugin.web_url.is_some() || plugin.link_url.is_some(),
            }),
        TabRef::PluginPath { plugin_id, .. } => plugins
            .installed
            .as_ref()
            .and_then(|list| list.plugins.iter().find(|plugin| plugin.id == *plugin_id))
            .is_some_and(|plugin| plugin.web_url.is_some()),
        TabRef::Bot { .. } | TabRef::SharedApp { .. } => true,
        TabRef::Widget { .. } => super::widget::available(tab, plugins),
        TabRef::Chat | TabRef::Apps => false,
    }
}

pub(super) fn plugin_is_running(
    state: &PagesState,
    tab: &TabRef,
    plugins: &PluginsViewModel,
) -> bool {
    if let TabRef::Bot { bot, .. } = tab {
        return bot::ready_page(state, bot).is_some();
    }
    let plugin_id = match tab {
        TabRef::Plugin { plugin_id, .. }
        | TabRef::PluginPath { plugin_id, .. }
        | TabRef::UiResource { plugin_id, .. }
        | TabRef::Widget { plugin_id, .. } => plugin_id,
        _ => return false,
    };
    plugins
        .installed
        .as_ref()
        .and_then(|list| list.plugins.iter().find(|plugin| plugin.id == *plugin_id))
        .is_some_and(|plugin| matches!(plugin.status, PluginState::Running { .. }))
}

pub(super) fn item_title(item: &StoredItem) -> String {
    if let TabRef::Bot { bot, .. } = &item.tab {
        return item
            .page
            .as_ref()
            .map(|page| page.plugin_title.clone())
            .unwrap_or_else(|| bot.bot.clone());
    }
    item.page
        .as_ref()
        .expect("plugin tab item must keep its page metadata")
        .plugin_title
        .clone()
}

/// 这个实例指的是哪个页面。PlacedTab 在 protocol 里，这里放个取值的自由函数。
pub(super) fn placed_tab_ref(placed: &PlacedTab) -> &TabRef {
    match placed {
        PlacedTab::Page { tab } | PlacedTab::Content { tab } => tab,
    }
}

pub(super) fn normalize_layout(layout: TabLayout) -> TabLayout {
    if layout == TabLayout::Other {
        TabLayout::Inset
    } else {
        layout
    }
}

/// 是网页的页面：插件页、插件里的某一页、公网网页、朋友分享的插件页。加载失败和重新加载都按网页处理。
pub(super) fn is_web_page(tab: &TabRef) -> bool {
    matches!(
        tab,
        TabRef::Plugin { .. }
            | TabRef::PluginPath { .. }
            | TabRef::UiResource { .. }
            | TabRef::Bot { .. }
            | TabRef::Url { .. }
            | TabRef::SharedApp { .. }
            | TabRef::Widget { .. }
    )
}
