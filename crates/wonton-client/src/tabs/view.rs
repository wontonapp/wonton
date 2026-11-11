//! 底栏和面板的 ViewModel 的页面和状态映射。

use url::Url;
use wonton_protocol::{
    CandidatePlugin, CoreError, InstalledPlugin, PanelEntry, PanelItem, PanelScope, PanelSource,
    PlacedTab, PluginState, PluginsViewModel, TabCandidate, TabDisplay, TabDisplayChoice, TabIcon,
    TabItem, TabLayout, TabPage, TabRef, TabState, TabsProblem, TabsViewModel,
};

use super::{Fullscreen, StoredItem, StoredPage, TabsState, invalid};
use crate::l10n::{CATALOG, language};
use wonton_l10n::l10n;

mod panel;
pub(crate) use panel::*;

pub(super) fn make_view(state: &TabsState, plugins: &PluginsViewModel) -> TabsViewModel {
    let items: Vec<_> = state
        .config
        .items
        .iter()
        .map(|item| build_item(state, item, plugins))
        .collect();
    let mut pages = Vec::with_capacity(items.len() + 1);
    pages.extend(items.iter().map(|item| item.tab.clone()));
    if !pages.contains(&TabRef::Apps) {
        pages.push(TabRef::Apps);
    }
    let show_bar = items.len() > 1;
    let overlay = state
        .overlay
        .as_ref()
        .map(|item| build_item(state, item, plugins));
    let main = PanelScope::Main;
    TabsViewModel {
        pages,
        items,
        show_bar,
        selected: state.selected.clone(),
        overlay,
        full: state.config.items.len() >= super::MAX_ITEMS,
        uninstalled: state.uninstalled.clone(),
        problem: state.problem.clone().or_else(|| {
            state
                .pins_problem
                .clone()
                .map(|message| TabsProblem { message })
        }),
        labels: state.config.labels,
        candidates: candidates(state, plugins),
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
        bot_pin_consents: state.bot_pin_consents.clone(),
    }
}

/// 设置在前，然后按已装插件的顺序列出每个能打开的页面，去掉已经在栏上的。没有网页的插件没有页面可放。
fn candidates(state: &TabsState, plugins: &PluginsViewModel) -> Vec<TabCandidate> {
    if state.external {
        return Vec::new();
    }
    std::iter::once(TabCandidate {
        tab: TabRef::Settings,
        plugin: None,
    })
    .chain(plugin_pages(plugins))
    .chain(super::sharing::candidates(plugins))
    .filter(|candidate| {
        !state
            .config
            .items
            .iter()
            .any(|item| item.tab == candidate.tab)
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

/// 底栏上的一项（含盖在主界面上的页面覆盖层）。
fn build_item(state: &TabsState, stored: &StoredItem, plugins: &PluginsViewModel) -> TabItem {
    let placed = PlacedTab::Bar {
        tab: stored.tab.clone(),
    };
    let page = match &stored.tab {
        TabRef::Chat | TabRef::Apps | TabRef::Settings => None,
        TabRef::Bot { .. } | TabRef::UiResource { .. } => None,
        TabRef::SharedApp { .. } => Some(super::sharing::page(state, stored, plugins)),
        TabRef::Plugin { plugin_id, page } => {
            let plugin = plugins
                .installed
                .as_ref()
                .and_then(|list| list.plugins.iter().find(|plugin| plugin.id == *plugin_id));
            Some(build_page(
                state,
                &placed,
                stored,
                plugin,
                page.as_deref(),
                plugins,
                None,
            ))
        }
        // 钉上来的插件里的某一页：插件在就照插件页合成，不在了按缓存的标题显示。
        TabRef::PluginPath { plugin_id, .. } => {
            let plugin = plugins
                .installed
                .as_ref()
                .and_then(|list| list.plugins.iter().find(|plugin| plugin.id == *plugin_id));
            Some(build_page(
                state, &placed, stored, plugin, None, plugins, None,
            ))
        }
        // 公网网页：没有插件，标题按域名，失败按手机自己的网络算。
        TabRef::Url { .. } => Some(build_page(
            state, &placed, stored, None, None, plugins, None,
        )),
        // MCP Apps 界面：栏上不放（Bot 只钉面板），照插件页显示。
        TabRef::UiResource { plugin_id, .. } => {
            let plugin = plugins
                .installed
                .as_ref()
                .and_then(|list| list.plugins.iter().find(|plugin| plugin.id == *plugin_id));
            Some(build_page(
                state, &placed, stored, plugin, None, plugins, None,
            ))
        }
    };
    let badge = match (&stored.tab, &page) {
        (TabRef::Settings, _) => plugins.core_upgrade.is_prompting(),
        (_, Some(page)) => matches!(page.state, TabState::Failed { .. } | TabState::PageRemoved),
        _ => false,
    };
    TabItem {
        tab: stored.tab.clone(),
        removable: is_removable(&stored.tab),
        badge,
        page,
    }
}

/// 公网网页的一项：网址照原样，非 https 的不打开并说清原因。房间账号数据由别的客户端直接写，
/// 不经过入口校验，所以加载时也查一遍（ADR-0052 只收 `https://`）。
fn build_url_page(
    state: &TabsState,
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
        TabDisplayChoice::Fullscreen,
        plugins,
    )
}

fn build_page(
    state: &TabsState,
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
            return page_from_metadata(
                state,
                placed,
                metadata,
                url,
                state_value,
                TabDisplayChoice::Fullscreen,
                plugins,
            );
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
            return page_from_metadata(
                state,
                placed,
                metadata,
                url,
                state_value,
                TabDisplayChoice::Fullscreen,
                plugins,
            );
        }
        TabRef::Url { url } => return build_url_page(state, placed, stored, url, plugins),
        _ => {}
    }
    let choice = display_for(state, &stored.tab);
    let Some(plugin) = plugin else {
        let metadata = stored
            .page
            .clone()
            .expect("plugin tab item must keep its page metadata");
        return page_from_metadata(
            state,
            placed,
            metadata,
            None,
            TabState::Loading,
            choice,
            plugins,
        );
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
            choice,
            plugins,
        );
    };
    let declared = page_id.and_then(|id| plugin.tabs.iter().find(|tab| tab.id == id));
    let path = declared.map(|page| page.path.as_str());
    let url = page_url(plugin.web_url.as_deref(), path);
    let state_value = page_state(state, placed, plugin, url.is_some());
    page_from_metadata(state, placed, metadata, url, state_value, choice, plugins)
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
fn path_title(path: &str) -> String {
    path.split(['?', '#'])
        .next()
        .unwrap_or(path)
        .rsplit('/')
        .find(|segment| !segment.is_empty())
        .unwrap_or(path)
        .to_owned()
}

pub(super) fn page_from_metadata(
    state: &TabsState,
    placed: &PlacedTab,
    metadata: StoredPage,
    url: Option<String>,
    state_value: TabState,
    choice: TabDisplayChoice,
    plugins: &PluginsViewModel,
) -> TabPage {
    let dimmed = matches!(state_value, TabState::Stopped);
    TabPage {
        title: metadata.title,
        icon: metadata.icon,
        url,
        layout: normalize_layout(metadata.layout),
        state: state_value,
        dimmed,
        display: choice,
        opens_fullscreen: opens_fullscreen(placed_tab_ref(placed), choice, plugins),
        fullscreen: state
            .fullscreen
            .as_ref()
            .is_some_and(|full| full.tab == *placed_tab_ref(placed)),
        reload: state.reloads.get(placed).copied().unwrap_or_default(),
    }
}

pub(super) fn page_state(
    state: &TabsState,
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
        };
    }
    match &plugin.status {
        PluginState::Installing => TabState::Starting,
        PluginState::Stopped => TabState::Stopped,
        PluginState::Running { .. } => TabState::Ready,
        PluginState::Upgrading => TabState::Upgrading,
        PluginState::Uninstalling => TabState::Uninstalling,
        PluginState::Degraded { reason } => TabState::Failed {
            reason: reason.clone(),
        },
        PluginState::Unknown => TabState::Unknown,
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
        TabRef::Settings => Ok(StoredItem {
            tab: tab.clone(),
            page: None,
        }),
        TabRef::Chat | TabRef::Apps => Err(invalid(&l10n!(
            CATALOG,
            language(),
            "tabs-view-always-pinned"
        ))),
        TabRef::SharedApp { .. } => super::sharing::stored(tab, plugins),
        TabRef::Bot { .. } | TabRef::UiResource { .. } => Err(invalid(&l10n!(
            CATALOG,
            language(),
            "tabs-view-unknown-plugin"
        ))),
        TabRef::Url { url } => {
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
                    plugin_title: host.clone().unwrap_or_else(|| url.clone()),
                    title: title.or(host).unwrap_or_else(|| url.clone()),
                    icon: TabIcon::Plugin { icon: None },
                    layout: TabLayout::Inset,
                }),
            })
        }
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

pub(super) fn set_automatic_fullscreen(
    state: &mut TabsState,
    item: &StoredItem,
    previous: TabRef,
    plugins: &PluginsViewModel,
) {
    if is_plugin(&item.tab) && opens_fullscreen(&item.tab, display_for(state, &item.tab), plugins) {
        state.fullscreen = Some(Fullscreen {
            tab: item.tab.clone(),
            return_to: Some(previous),
        });
    } else {
        state.fullscreen = None;
    }
}

pub(super) fn opens_fullscreen(
    tab: &TabRef,
    choice: TabDisplayChoice,
    plugins: &PluginsViewModel,
) -> bool {
    if !page_available(tab, plugins) {
        return false;
    }
    match choice {
        TabDisplayChoice::Fullscreen => true,
        TabDisplayChoice::Standard => false,
        TabDisplayChoice::Default => manifest_display(tab, plugins)
            .is_some_and(|display| matches!(display, TabDisplay::Fullscreen)),
    }
}

fn manifest_display(tab: &TabRef, plugins: &PluginsViewModel) -> Option<TabDisplay> {
    if matches!(tab, TabRef::SharedApp { .. }) {
        return super::sharing::shared_row(tab, plugins).map(|(_, row)| row.plugin.display);
    }
    let TabRef::Plugin { plugin_id, page } = tab else {
        return None;
    };
    plugins
        .installed
        .as_ref()
        .and_then(|list| list.plugins.iter().find(|plugin| plugin.id == *plugin_id))
        .and_then(|plugin| match page {
            Some(page) => plugin
                .tabs
                .iter()
                .find(|tab| tab.id == *page)
                .map(|tab| tab.display),
            None => Some(plugin.web_display),
        })
}

pub(super) fn page_available(tab: &TabRef, plugins: &PluginsViewModel) -> bool {
    if matches!(tab, TabRef::SharedApp { .. }) {
        return super::sharing::usable(tab, plugins);
    }
    let TabRef::Plugin { plugin_id, page } = tab else {
        return false;
    };
    plugins
        .installed
        .as_ref()
        .and_then(|list| list.plugins.iter().find(|plugin| plugin.id == *plugin_id))
        .is_some_and(|plugin| match page {
            Some(page) => plugin.tabs.iter().any(|tab| tab.id == *page),
            None => plugin.web_url.is_some(),
        })
}

pub(super) fn display_for(state: &TabsState, tab: &TabRef) -> TabDisplayChoice {
    state
        .config
        .displays
        .iter()
        .find(|entry| entry.tab == *tab)
        .map(|entry| entry.display)
        .unwrap_or(TabDisplayChoice::Default)
}

pub(super) fn plugin_is_running(tab: &TabRef, plugins: &PluginsViewModel) -> bool {
    let plugin_id = match tab {
        TabRef::Plugin { plugin_id, .. }
        | TabRef::PluginPath { plugin_id, .. }
        | TabRef::UiResource { plugin_id, .. } => plugin_id,
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
        PlacedTab::Bar { tab } | PlacedTab::Panel { tab, .. } => tab,
    }
}

pub(super) fn normalize_layout(layout: TabLayout) -> TabLayout {
    if layout == TabLayout::Other {
        TabLayout::Inset
    } else {
        layout
    }
}

pub(super) fn is_plugin(tab: &TabRef) -> bool {
    matches!(tab, TabRef::Plugin { .. } | TabRef::SharedApp { .. })
}

/// 绑在插件上的页面：插件页和插件里的某一页。换服务器时它们跟着走，公网网页不绑服务器。
pub(super) fn is_plugin_page(tab: &TabRef) -> bool {
    matches!(
        tab,
        TabRef::Plugin { .. }
            | TabRef::PluginPath { .. }
            | TabRef::SharedApp { .. }
            | TabRef::UiResource { .. }
    )
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
    )
}

pub(super) fn is_removable(tab: &TabRef) -> bool {
    // “应用”只在没配对时在栏上，那时它是配对页，也不能拿。
    !matches!(tab, TabRef::Chat | TabRef::Apps)
}
pub(crate) mod rules;
pub(crate) use rules::*;
