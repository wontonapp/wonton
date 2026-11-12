use super::*;
use wonton_protocol::{
    BotRef, BotWebFailure, BotWebPage, BotWebView, TabLayout, TabPage, TabState,
};

pub(super) fn ready_page<'a>(state: &'a PagesState, bot: &BotRef) -> Option<&'a BotWebPage> {
    let view = state.bots.iter().find(|view| view.bot == *bot)?;
    let BotWebView::Ready { page } = view.web.as_ref()? else {
        return None;
    };
    if page.bot != *bot {
        return None;
    }
    let url = Url::parse(&page.url).ok()?;
    (matches!(url.scheme(), "http" | "https") && url.host_str().is_some()).then_some(page)
}

pub(super) fn resolved_url(page: &BotWebPage, path: Option<&str>) -> Option<String> {
    if path.is_some_and(|path| Url::parse(path).is_ok()) {
        return None;
    }
    page_url(Some(&page.url), path)
}

pub(super) fn stored_item_or_plugin(
    state: &PagesState,
    tab: &TabRef,
    plugins: &PluginsViewModel,
    title: Option<String>,
) -> Result<StoredItem, CoreError> {
    if matches!(tab, TabRef::Bot { .. }) {
        stored_item(state, tab, title)
    } else {
        stored_item_with_title(state, tab, plugins, title)
    }
}

pub(super) fn replace_bots(state: &mut PagesState, bots: Vec<BotView>) {
    if state.bots == bots {
        return;
    }
    // A revoked or replaced grant invalidates every document instance, including panel copies.
    let changed: HashSet<_> = state
        .bots
        .iter()
        .chain(&bots)
        .filter_map(|view| {
            let old = state
                .bots
                .iter()
                .find(|old| old.bot == view.bot)
                .and_then(|old| old.web.as_ref());
            let new = bots
                .iter()
                .find(|new| new.bot == view.bot)
                .and_then(|new| new.web.as_ref());
            (old != new).then(|| view.bot.clone())
        })
        .collect();
    state.failures.retain(|placed, _| !matches!(placed_tab_ref(placed), TabRef::Bot { bot, .. } if changed.contains(bot)));
    let placed: Vec<_> = state
        .stack
        .iter()
        .map(|item| PlacedTab::Page {
            tab: item.tab.clone(),
        })
        .chain(state.config.panel.iter().map(|entry| PlacedTab::Panel {
            scope: PanelScope::Main,
            tab: entry.tab.clone(),
        }))
        .chain(state.open_room.iter().flat_map(|(room, panel)| {
            panel.pages.iter().map(move |entry| PlacedTab::Panel {
                scope: PanelScope::Room {
                    room_id: room.to_string(),
                },
                tab: entry.tab.clone(),
            })
        }))
        .collect();
    for placed in placed {
        if matches!(placed_tab_ref(&placed), TabRef::Bot { bot, .. } if changed.contains(bot)) {
            increment_reload(state, &placed);
        }
    }
    state.bots = bots;
}

fn metadata(state: &PagesState, bot: &BotRef, title: Option<String>) -> StoredPage {
    let name = state
        .bots
        .iter()
        .find(|view| view.bot == *bot)
        .map(|view| view.name.clone())
        .unwrap_or_else(|| bot.bot.clone());
    StoredPage {
        plugin_title: name.clone(),
        title: title.unwrap_or(name),
        icon: TabIcon::Plugin { icon: None },
        layout: TabLayout::Inset,
    }
}

pub(super) fn stored_item(
    state: &PagesState,
    tab: &TabRef,
    title: Option<String>,
) -> Result<StoredItem, CoreError> {
    let TabRef::Bot { bot, path } = tab else {
        unreachable!("Bot tab required")
    };
    let page = ready_page(state, bot).ok_or_else(|| CoreError::NotReady {
        reason: l10n!(CATALOG, language(), "tabs-view-plugin-no-page"),
    })?;
    if path.as_ref().is_some_and(|path| Url::parse(path).is_ok()) {
        return Err(invalid(&l10n!(
            CATALOG,
            language(),
            "panel-pin-path-outside"
        )));
    }
    let url = resolved_url(page, path.as_deref())
        .ok_or_else(|| invalid(&l10n!(CATALOG, language(), "panel-pin-path-outside")))?;
    let path = path
        .as_ref()
        .map(|_| relative_path(&Url::parse(&url).expect("validated page URL")));
    Ok(StoredItem {
        tab: TabRef::Bot {
            bot: bot.clone(),
            path,
        },
        page: Some(metadata(state, bot, title)),
    })
}

pub(super) fn build_page(
    state: &PagesState,
    placed: &PlacedTab,
    stored: &StoredItem,
    plugins: &PluginsViewModel,
) -> TabPage {
    let TabRef::Bot { bot, path } = &stored.tab else {
        unreachable!("Bot tab required")
    };
    let ready = ready_page(state, bot);
    let url = ready.and_then(|page| resolved_url(page, path.as_deref()));
    let browser = ready.is_some_and(|page| {
        url.is_some()
            && crate::browser::opens_in_browser_bot(
                page.cross_origin_isolated,
                crate::browser::ENGINE,
            )
    });
    let view = state.bots.iter().find(|view| view.bot == *bot);
    let state_value = if !state.connected {
        TabState::Loading
    } else if url.is_some() {
        state
            .failures
            .get(placed)
            .map_or(TabState::Ready, |(failure, status)| TabState::LoadFailed {
                failure: *failure,
                status: *status,
            })
    } else {
        match view.and_then(|view| view.web.as_ref()) {
            Some(BotWebView::Unavailable {
                reason: BotWebFailure::NotReady,
            }) => TabState::Starting,
            Some(BotWebView::Unavailable {
                reason: BotWebFailure::NotFound | BotWebFailure::NoWebui,
            }) => TabState::PageRemoved,
            _ => TabState::Unknown,
        }
    };
    let metadata = stored
        .page
        .clone()
        .unwrap_or_else(|| metadata(state, bot, None));
    let _ = plugins;
    TabPage {
        title: metadata.title,
        icon: metadata.icon,
        url,
        layout: metadata.layout,
        state: state_value,
        reload: state.reloads.get(placed).copied().unwrap_or_default(),
        opens_in_browser: browser,
    }
}

pub(super) fn classify_pin(state: &PagesState, parsed: &Url) -> Option<(TabRef, Option<String>)> {
    state.bots.iter().find_map(|view| {
        let page = ready_page(state, &view.bot)?;
        let base = Url::parse(&page.url).ok()?;
        if parsed.origin() != base.origin() {
            return None;
        }
        Some((
            TabRef::Bot {
                bot: view.bot.clone(),
                path: Some(relative_path(parsed)),
            },
            Some(view.name.clone()),
        ))
    })
}

fn relative_path(url: &Url) -> String {
    let mut path = url.path().to_owned();
    if let Some(query) = url.query() {
        path.push('?');
        path.push_str(query);
    }
    if let Some(fragment) = url.fragment() {
        path.push('#');
        path.push_str(fragment);
    }
    path
}
