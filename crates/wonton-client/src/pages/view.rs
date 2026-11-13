//! 页面栈、我的应用和面板的 ViewModel 映射。

use url::Url;
use wonton_protocol::{
    CandidatePlugin, CoreError, InstalledPlugin, MyAppRef, OpenPage, PagesProblem, PagesViewModel,
    PanelEntry, PanelItem, PanelScope, PanelSource, PlacedTab, PluginState, PluginsViewModel,
    SidebarApp, SidebarBadge, SidebarOpen, TabCandidate, TabIcon, TabLayout, TabLoadFailure,
    TabPage, TabRef, TabState,
};

use super::panel_view::main_panel;
use super::{PagesState, StoredApp, StoredItem, StoredPage, invalid};
use crate::l10n::{CATALOG, language};
use wonton_l10n::l10n;

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
    let main = PanelScope::Main;
    PagesViewModel {
        root: state.root(),
        stack,
        uninstalled: state.uninstalled.clone(),
        // 钉页拉不到（连着却出错）也如实报；已知连不上时 pins 那边不记，只有连接提示和侧边栏的轻提示。
        problem: state.problem.clone().or_else(|| {
            state
                .pins_problem
                .clone()
                .map(|message| PagesProblem { message })
        }),
        my_app_candidates: my_app_candidates(state, plugins),
        auto_add: state.config.auto_add,
        bot_pin_consents: state.bot_pin_consents.clone(),
        panel: main_panel(state, plugins),
        panel_candidates: plugin_pages(plugins)
            .filter(|candidate| {
                !state
                    .config
                    .panel
                    .iter()
                    .any(|entry| entry.tab == candidate.tab)
                    && !state
                        .bot_pins
                        .iter()
                        .any(|pin| pin.scope == main && pin.tab == candidate.tab)
            })
            .collect(),
        panel_selected: state.panel_selected.clone(),
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
    let (plugin_id, tab, title, icon, open) = match &entry.app {
        MyAppRef::Plugin { plugin_id } => match plugin(plugin_id) {
            Some(plugin) => {
                let open = if plugin.kind == wonton_protocol::PluginKind::Agent
                    && let Some(bot_user_id) = plugin.bot_user_id.clone()
                {
                    SidebarOpen::BotChat { bot_user_id }
                } else if plugin.web_url.is_some() {
                    SidebarOpen::Page {
                        tab: TabRef::Plugin {
                            plugin_id: plugin_id.clone(),
                            page: plugin.tabs.first().map(|page| page.id.clone()),
                        },
                    }
                } else {
                    return None;
                };
                (
                    Some(plugin_id.clone()),
                    None,
                    entry.title.clone().unwrap_or_else(|| plugin.title.clone()),
                    plugin.icon.clone(),
                    open,
                )
            }
            // 列表没读到：按存的元数据显示（真被卸载的行在卸载时就拿掉了）。
            // Agent 行没有存的 bot 账号，显示不了，先空着。
            None => {
                let page = entry.page.as_ref()?;
                (
                    Some(plugin_id.clone()),
                    None,
                    entry.title.clone().unwrap_or_else(|| page.title.clone()),
                    match &page.icon {
                        TabIcon::Plugin { icon } => icon.clone(),
                        TabIcon::Monochrome { .. } => None,
                    },
                    SidebarOpen::Page {
                        tab: TabRef::Plugin {
                            plugin_id: plugin_id.clone(),
                            page: None,
                        },
                    },
                )
            }
        },
        MyAppRef::Page { tab } => {
            let page = entry.page.clone()?;
            let plugin_id = match tab {
                TabRef::Plugin { plugin_id, .. } | TabRef::PluginPath { plugin_id, .. } => {
                    Some(plugin_id.clone())
                }
                _ => None,
            };
            let _live_problem = state
                .failures
                .contains_key(&PlacedTab::Page { tab: tab.clone() });
            (
                plugin_id,
                Some(tab.clone()),
                entry.title.clone().unwrap_or(page.title),
                match &page.icon {
                    TabIcon::Plugin { icon } => icon.clone(),
                    TabIcon::Monochrome { .. } => None,
                },
                SidebarOpen::Page { tab: tab.clone() },
            )
        }
    };
    let page_tab = tab.or_else(|| match &open {
        SidebarOpen::Page { tab } => Some(tab.clone()),
        SidebarOpen::BotChat { .. } => None,
    });
    let plugin = plugin_id.as_deref().and_then(plugin);
    let problem = problem_badge(entry, page_tab.as_ref(), plugin, open_pages);
    Some(SidebarApp {
        plugin_id: plugin_id.clone(),
        tab: page_tab,
        title,
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
        },
    })
}

/// 页面出了问题要不要红点：插件坏了（降级）、或这一行对应的打开页面被新版本移走。
fn problem_badge(
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
                    super::tab_plugin_id(&open.tab) == Some(plugin_id.as_str())
                }
                _ => false,
            },
        }
    };
    open_pages
        .iter()
        .any(|open| matches(open) && open.page.state == TabState::PageRemoved)
}

/// 还没加进我的应用的插件页，按已装插件的顺序。没有网页的插件没有页面可加。
fn my_app_candidates(state: &PagesState, plugins: &PluginsViewModel) -> Vec<TabCandidate> {
    plugin_pages(plugins)
        .filter(|candidate| match &candidate.tab {
            // 插件行盖住这个插件的全部页面；只加了别的页面行时，这一页还能加。
            TabRef::Plugin { plugin_id, page: Some(_) } => {
                !state.config.has_plugin_row(plugin_id)
                    && state
                        .config
                        .position(&MyAppRef::Page {
                            tab: candidate.tab.clone(),
                        })
                        .is_none()
            }
            // 插件级入口：这个插件有任何行都不再重复给。
            _ => {
                let app = match &candidate.tab {
                    TabRef::Plugin { plugin_id, .. } => MyAppRef::Plugin {
                        plugin_id: plugin_id.clone(),
                    },
                    _ => MyAppRef::Page {
                        tab: candidate.tab.clone(),
                    },
                };
                state.config.position(&app).is_none()
                    && !matches!(&candidate.tab, TabRef::Plugin { plugin_id, .. } if state.config.has_plugin(plugin_id))
            }
        })
        .collect()
}

/// 每个能打开的插件页：声明了 `tabs` 的按页，没声明的是网页入口。没有网页的插件没有页面。
fn plugin_pages(plugins: &PluginsViewModel) -> impl Iterator<Item = TabCandidate> + '_ {
    let installed = plugins.installed.iter().flat_map(|list| &list.plugins);
    installed
        .filter(|plugin| plugin.web_url.is_some())
        .flat_map(|plugin| {
            let candidate = |page: Option<&wonton_protocol::PluginTab>| TabCandidate {
                tab: TabRef::Plugin {
                    plugin_id: plugin.id.clone(),
                    page: page.map(|page| page.id.clone()),
                },
                plugin: Some(CandidatePlugin {
                    title: plugin.title.clone(),
                    page_title: page
                        .filter(|_| plugin.tabs.len() > 1)
                        .map(|page| page.title.clone()),
                    icon: plugin.icon.clone(),
                }),
            };
            if plugin.tabs.is_empty() {
                vec![candidate(None)]
            } else {
                plugin
                    .tabs
                    .iter()
                    .map(|page| candidate(Some(page)))
                    .collect()
            }
        })
}

/// 栈里的一页：页面元数据按插件或缓存合成。
fn build_page_item(state: &PagesState, stored: &StoredItem, plugins: &PluginsViewModel) -> TabPage {
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
    build_page(state, &placed, stored, plugin, page_id, plugins, None)
}

/// 面板里的一项。面板页照全屏渲染（ADR-0052：WebView 一直是第 2 级的大小）。
pub(super) fn build_panel_item(
    state: &PagesState,
    entry: &PanelEntry,
    scope: &PanelScope,
    plugins: &PluginsViewModel,
) -> PanelItem {
    let placed = PlacedTab::Panel {
        scope: scope.clone(),
        tab: entry.tab.clone(),
    };
    let plugin = match &entry.tab {
        TabRef::Plugin { plugin_id, .. } | TabRef::PluginPath { plugin_id, .. } => plugins
            .installed
            .as_ref()
            .and_then(|list| list.plugins.iter().find(|plugin| plugin.id == *plugin_id)),
        _ => None,
    };
    let page_id = match &entry.tab {
        TabRef::Plugin {
            page: Some(page), ..
        } => Some(page.as_str()),
        _ => None,
    };
    let mut page = build_page(
        state,
        &placed,
        &StoredItem {
            tab: entry.tab.clone(),
            page: None,
        },
        plugin,
        page_id,
        plugins,
        None,
    );
    if let Some(title) = &entry.title {
        page.title = title.clone();
    }
    PanelItem {
        tab: entry.tab.clone(),
        page,
        attention: state.attention.contains(&placed),
        source: PanelSource::User,
        movable: true,
    }
}

/// 公网网页的一项：网址照原样，非 https 的不打开并说清原因。房间账号数据由别的客户端直接写，
/// 不经过入口校验，所以加载时也查一遍（ADR-0052 只收 `https://`）。
fn build_url_page(
    state: &PagesState,
    placed: &PlacedTab,
    stored: &StoredItem,
    url: &str,
    plugins: &PluginsViewModel,
) -> TabPage {
    let host = || {
        url.parse::<Url>()
            .ok()
            .and_then(|parsed| parsed.host_str().map(str::to_owned))
            .unwrap_or_else(|| url.to_owned())
    };
    let metadata = stored.page.clone().unwrap_or_else(|| StoredPage {
        plugin_title: host(),
        title: host(),
        icon: TabIcon::Plugin { icon: None },
        layout: TabLayout::Inset,
    });
    // 公网网页不经隧道，连不上是手机自己的网络，按上游出错显示（ADR-0052）。
    let https = is_https_url(url);
    let state_value = if https {
        match state.failures.get(placed) {
            Some((failure, status)) => TabState::LoadFailed {
                failure: *failure,
                status: *status,
                details: load_failed_details(*failure, *status),
            },
            None => TabState::Ready,
        }
    } else {
        TabState::Failed {
            reason: l10n!(CATALOG, language(), "panel-pin-not-https"),
        }
    };
    page_from_metadata(
        state,
        placed,
        metadata,
        https.then(|| url.to_owned()),
        state_value,
        plugins,
    )
}

/// 本机插件的页面要跨源隔离、这个平台的 WebView 给不了时交给系统浏览器（ADR-0033）。
pub(super) fn build_page(
    state: &PagesState,
    placed: &PlacedTab,
    stored: &StoredItem,
    plugin: Option<&InstalledPlugin>,
    page_id: Option<&str>,
    plugins: &PluginsViewModel,
    pin: Option<&wonton_protocol::BotPanelPin>,
) -> TabPage {
    let page = compose_page(state, placed, stored, plugin, page_id, plugins, pin);
    let own_page = matches!(
        stored.tab,
        TabRef::Plugin { .. } | TabRef::PluginPath { .. }
    );
    let browser =
        plugin.is_some_and(|p| crate::browser::opens_in_browser(p, crate::browser::ENGINE));
    TabPage {
        opens_in_browser: own_page && browser,
        ..page
    }
}

fn compose_page(
    state: &PagesState,
    placed: &PlacedTab,
    stored: &StoredItem,
    plugin: Option<&InstalledPlugin>,
    page_id: Option<&str>,
    plugins: &PluginsViewModel,
    pin: Option<&wonton_protocol::BotPanelPin>,
) -> TabPage {
    match &stored.tab {
        // 插件里的某一页和公网网页：网址核心直接拼，插件不在了还能用缓存的标题显示。
        TabRef::PluginPath { plugin_id, path } => {
            let url = plugin.and_then(|plugin| page_url(plugin.web_url.as_deref(), Some(path)));
            let state_value = match plugin {
                Some(plugin) if url.is_some() => page_state(state, placed, plugin, true),
                _ => TabState::PageRemoved,
            };
            let metadata = stored.page.clone().unwrap_or_else(|| StoredPage {
                plugin_title: plugin_title_of(plugin_id, plugins),
                title: path_title(path),
                icon: TabIcon::Plugin {
                    icon: plugin.and_then(|plugin| plugin.icon.clone()),
                },
                layout: TabLayout::Inset,
            });
            return page_from_metadata(state, placed, metadata, url, state_value, plugins);
        }
        // MCP Apps 界面：宿主页由核心给，界面放在它的 iframe 里。
        TabRef::UiResource { plugin_id, .. } => {
            let url = pin
                .zip(state.core_url.as_ref())
                .and_then(|(pin, base)| {
                    base.join(&wonton_protocol::mcp_app_host_path(&pin.id)).ok()
                })
                .map(|url| url.to_string());
            let state_value = if url.is_some() {
                match state.failures.get(placed) {
                    Some((failure, status)) => TabState::LoadFailed {
                        failure: *failure,
                        status: *status,
                        details: load_failed_details(*failure, *status),
                    },
                    None => TabState::Ready,
                }
            } else {
                TabState::PageRemoved
            };
            let metadata = StoredPage {
                plugin_title: plugin_title_of(plugin_id, plugins),
                title: pin.map(|pin| pin.title.clone()).unwrap_or_default(),
                icon: TabIcon::Plugin {
                    icon: plugin.and_then(|plugin| plugin.icon.clone()),
                },
                layout: TabLayout::EdgeToEdge,
            };
            return page_from_metadata(state, placed, metadata, url, state_value, plugins);
        }
        TabRef::Url { url } => return build_url_page(state, placed, stored, url, plugins),
        _ => {}
    }
    let Some(plugin) = plugin else {
        let metadata = stored
            .page
            .clone()
            .expect("plugin tab item must keep its page metadata");
        return page_from_metadata(state, placed, metadata, None, TabState::Loading, plugins);
    };
    let Some(metadata) = metadata_for_plugin(plugin, page_id) else {
        let metadata = stored
            .page
            .clone()
            .expect("plugin tab item must keep its page metadata");
        return page_from_metadata(
            state,
            placed,
            metadata,
            None,
            TabState::PageRemoved,
            plugins,
        );
    };
    let declared = page_id.and_then(|id| plugin.tabs.iter().find(|tab| tab.id == id));
    let path = declared.map(|page| page.path.as_str());
    let url = page_url(plugin.web_url.as_deref(), path);
    let state_value = page_state(state, placed, plugin, url.is_some());
    page_from_metadata(state, placed, metadata, url, state_value, plugins)
}

fn plugin_title_of(plugin_id: &str, plugins: &PluginsViewModel) -> String {
    plugins
        .installed
        .as_ref()
        .and_then(|list| list.plugins.iter().find(|plugin| plugin.id == plugin_id))
        .map(|plugin| plugin.title.clone())
        .unwrap_or_else(|| plugin_id.to_owned())
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
fn load_failed_details(failure: TabLoadFailure, status: Option<u16>) -> Option<String> {
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
        return plugin.web_url.as_ref().map(|_| StoredPage {
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
    Err(invalid(&l10n!(
        CATALOG,
        language(),
        "panel-pin-path-outside"
    )))
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

pub(super) fn stored_item_for_tab(
    tab: &TabRef,
    plugins: &PluginsViewModel,
) -> Result<StoredItem, CoreError> {
    stored_item_with_title(tab, plugins, None)
}

/// 钉页面上来的带用户起的名字；候选页来的没有，按插件或域名补。
pub(super) fn stored_item_with_title(
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
        TabRef::Bot { bot, .. } => Ok(StoredItem {
            tab: tab.clone(),
            page: Some(StoredPage {
                plugin_title: bot.bot.clone(),
                title: bot.bot.clone(),
                icon: TabIcon::Plugin { icon: None },
                layout: TabLayout::Inset,
            }),
        }),
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

fn stored_url_item(
    tab: &TabRef,
    url: &str,
    title: Option<String>,
) -> Result<StoredItem, CoreError> {
    if !is_https_url(url) {
        return Err(invalid(&l10n!(CATALOG, language(), "panel-pin-not-https")));
    }
    let host = url
        .parse::<Url>()
        .ok()
        .and_then(|parsed| parsed.host_str().map(str::to_owned));
    Ok(StoredItem {
        tab: tab.clone(),
        page: Some(StoredPage {
            plugin_title: host.clone().unwrap_or_else(|| url.to_owned()),
            title: title.or(host).unwrap_or_else(|| url.to_owned()),
            icon: TabIcon::Plugin { icon: None },
            layout: TabLayout::Inset,
        }),
    })
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
                None => plugin.web_url.is_some(),
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

pub(super) fn plugin_is_running(tab: &TabRef, plugins: &PluginsViewModel) -> bool {
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
    item.page
        .as_ref()
        .expect("plugin tab item must keep its page metadata")
        .plugin_title
        .clone()
}

/// 这个实例指的是哪个页面。PlacedTab 在 protocol 里，这里放个取值的自由函数。
pub(super) fn placed_tab_ref(placed: &PlacedTab) -> &TabRef {
    match placed {
        PlacedTab::Page { tab } | PlacedTab::Panel { tab, .. } => tab,
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
            | TabRef::Url { .. }
            | TabRef::SharedApp { .. }
            | TabRef::Bot { .. }
            | TabRef::Widget { .. }
    )
}
