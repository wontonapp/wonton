//! 底栏的动作：选中、固定、移除、排序、打开方式、全屏、重新加载和加载结果。
//! 面板的动作在 [`panel`](super::panel)。
use super::*;

impl TabsLink {
    pub(crate) fn select(&self, tab: TabRef) -> Result<(), CoreError> {
        self.runtime_change(|state, plugins| {
            if state.selected == tab
                && state
                    .fullscreen
                    .as_ref()
                    .is_some_and(|fullscreen| fullscreen.tab == tab)
            {
                return Ok(());
            }
            if let Some(item) = state
                .config
                .items
                .iter()
                .find(|item| item.tab == tab)
                .cloned()
            {
                let previous = state.selected.clone();
                state.selected = tab;
                state.overlay = None;
                set_automatic_fullscreen(state, &item, previous, plugins);
                return Ok(());
            }
            // "应用"网格页只从侧边栏进，不在栏上也能选中：选它等于把网格页推到前面，栏照旧隐藏。
            if tab == TabRef::Apps {
                state.selected = tab;
                state.overlay = None;
                state.fullscreen = None;
                return Ok(());
            }
            if !is_plugin(&tab) {
                return Err(invalid(&l10n!(CATALOG, language(), "tabs-bar-not-pinned")));
            }
            let item = stored_item_for_tab(&tab, plugins)?;
            let previous = state.selected.clone();
            state.overlay = Some(item.clone());
            set_automatic_fullscreen(state, &item, previous, plugins);
            Ok(())
        })
    }

    pub(super) fn close_overlay(&self) -> Result<(), CoreError> {
        self.runtime_change(|state, _| {
            let overlay = state.overlay.take();
            if let Some(overlay) = overlay
                && state
                    .fullscreen
                    .as_ref()
                    .is_some_and(|full| full.tab == overlay.tab)
            {
                state.fullscreen = None;
            }
            Ok(())
        })
    }

    pub(super) fn pin(
        &self,
        tab: TabRef,
        replacing: Option<TabRef>,
        title: Option<String>,
    ) -> Result<(), CoreError> {
        self.config_change(|state, plugins| {
            if state.external {
                return Err(invalid("external Matrix account navigation is fixed"));
            }
            if state.config.items.iter().any(|item| item.tab == tab) {
                return Err(invalid(&l10n!(
                    CATALOG,
                    language(),
                    "tabs-bar-already-pinned"
                )));
            }
            let item = stored_item_with_title(&tab, plugins, title)?;
            if state.config.items.len() >= MAX_ITEMS {
                let Some(replacing) = replacing else {
                    return Err(invalid(&l10n!(
                        CATALOG,
                        language(),
                        "tabs-bar-replace-required"
                    )));
                };
                let Some(index) = state
                    .config
                    .items
                    .iter()
                    .position(|item| item.tab == replacing)
                else {
                    return Err(invalid(&l10n!(
                        CATALOG,
                        language(),
                        "tabs-bar-replacing-missing"
                    )));
                };
                if !is_removable(&replacing) {
                    return Err(invalid(&l10n!(CATALOG, language(), "tabs-bar-fixed")));
                }
                state.config.items[index] = item;
                state
                    .config
                    .displays
                    .retain(|display| display.tab != replacing);
                if state.selected == replacing {
                    state.selected = TabRef::Apps;
                    state.fullscreen = None;
                }
            } else {
                if replacing.is_some() {
                    return Err(invalid(&l10n!(CATALOG, language(), "tabs-bar-not-full")));
                }
                state.config.items.push(item);
            }
            if state
                .overlay
                .as_ref()
                .is_some_and(|overlay| overlay.tab == tab)
            {
                state.overlay = None;
                state.fullscreen = None;
            }
            Ok(())
        })
    }

    pub(super) fn unpin(&self, tab: TabRef) -> Result<(), CoreError> {
        self.config_change(|state, _| {
            if !is_removable(&tab) {
                return Err(invalid(&l10n!(
                    CATALOG,
                    language(),
                    "tabs-bar-remove-fixed"
                )));
            }
            let Some(index) = state.config.items.iter().position(|item| item.tab == tab) else {
                return Err(invalid(&l10n!(
                    CATALOG,
                    language(),
                    "tabs-bar-page-missing"
                )));
            };
            state.config.items.remove(index);
            state.config.displays.retain(|display| display.tab != tab);
            if state.selected == tab {
                state.selected = TabRef::Apps;
                state.fullscreen = None;
            }
            if state
                .overlay
                .as_ref()
                .is_some_and(|overlay| overlay.tab == tab)
            {
                state.overlay = None;
                state.fullscreen = None;
            }
            Ok(())
        })
    }

    pub(super) fn move_tab(&self, tab: TabRef, index: u32) -> Result<(), CoreError> {
        self.config_change(|state, _| {
            let index = index as usize;
            if index >= state.config.items.len() {
                return Err(invalid(&l10n!(
                    CATALOG,
                    language(),
                    "tabs-bar-index-out-of-range"
                )));
            }
            let Some(current) = state.config.items.iter().position(|item| item.tab == tab) else {
                return Err(invalid(&l10n!(
                    CATALOG,
                    language(),
                    "tabs-bar-page-missing"
                )));
            };
            let item = state.config.items.remove(current);
            state.config.items.insert(index, item);
            Ok(())
        })
    }

    pub(super) fn set_labels(&self, visible: bool) -> Result<(), CoreError> {
        self.config_change(|state, _| {
            state.config.labels = visible;
            Ok(())
        })
    }

    pub(super) fn set_display(
        &self,
        tab: TabRef,
        display: TabDisplayChoice,
    ) -> Result<(), CoreError> {
        self.config_change(|state, plugins| {
            if !is_plugin(&tab) {
                return Err(invalid(&l10n!(
                    CATALOG,
                    language(),
                    "tabs-open-only-plugins"
                )));
            }
            let _ = stored_item_for_tab(&tab, plugins)?;
            state.config.displays.retain(|entry| entry.tab != tab);
            if display != TabDisplayChoice::Default {
                state.config.displays.push(StoredDisplay {
                    tab: tab.clone(),
                    display,
                });
            }
            if state
                .fullscreen
                .as_ref()
                .is_some_and(|full| full.tab == tab)
                && !opens_fullscreen(&tab, display_for(state, &tab), plugins)
            {
                state.fullscreen = None;
            }
            Ok(())
        })
    }

    pub(super) fn enter_fullscreen(&self, tab: TabRef) -> Result<(), CoreError> {
        self.runtime_change(|state, plugins| {
            if !is_plugin(&tab) {
                return Err(invalid(&l10n!(
                    CATALOG,
                    language(),
                    "tabs-fullscreen-only-plugins"
                )));
            }
            if state
                .fullscreen
                .as_ref()
                .is_some_and(|fullscreen| fullscreen.tab == tab)
            {
                return Ok(());
            }
            if state.config.items.iter().any(|item| item.tab == tab) {
                state.selected = tab.clone();
                state.overlay = None;
            } else {
                state.overlay = Some(stored_item_for_tab(&tab, plugins)?);
            }
            state.fullscreen = Some(Fullscreen {
                tab,
                return_to: None,
            });
            Ok(())
        })
    }

    pub(super) fn exit_fullscreen(&self, tab: TabRef) -> Result<(), CoreError> {
        self.runtime_change(|state, _| {
            let Some(fullscreen) = state.fullscreen.take() else {
                return Err(invalid(&l10n!(CATALOG, language(), "tabs-fullscreen-none")));
            };
            if fullscreen.tab != tab {
                state.fullscreen = Some(fullscreen);
                return Err(invalid(&l10n!(
                    CATALOG,
                    language(),
                    "tabs-fullscreen-not-current"
                )));
            }
            if let Some(return_to) = fullscreen.return_to {
                state.overlay = None;
                state.selected = if state.config.items.iter().any(|item| item.tab == return_to) {
                    return_to
                } else {
                    state.config.items[0].tab.clone()
                };
            }
            Ok(())
        })
    }

    pub(super) fn reload(&self, page: PlacedTab) -> Result<(), CoreError> {
        self.runtime_change(|state, _| {
            if !is_web_page(placed_tab_ref(&page)) || !state_has_page(state, &page) {
                return Err(invalid(&l10n!(
                    CATALOG,
                    language(),
                    "tabs-page-cannot-reload"
                )));
            }
            state.failures.remove(&page);
            increment_reload(state, &page);
            Ok(())
        })
    }

    pub(crate) fn load_failed(
        &self,
        page: PlacedTab,
        status: Option<u16>,
    ) -> Result<(), CoreError> {
        self.runtime_change(|state, plugins| {
            if !is_web_page(placed_tab_ref(&page)) || !state_has_page(state, &page) {
                return Err(invalid(&l10n!(
                    CATALOG,
                    language(),
                    "tabs-bridge-page-not-open"
                )));
            }
            let tab = placed_tab_ref(&page);
            let failure = if matches!(tab, TabRef::Url { .. }) {
                // 公网网页不经隧道：连不上是手机自己的网络，拿到状态码也是上游出错（ADR-0052）。
                TabLoadFailure::Upstream
            } else if status.is_none() {
                TabLoadFailure::Transport
            } else if !plugin_is_running(tab, plugins) {
                TabLoadFailure::PluginNotReady
            } else {
                TabLoadFailure::Upstream
            };
            state.failures.insert(page, (failure, status));
            Ok(())
        })
    }

    pub(crate) fn load_succeeded(&self, page: PlacedTab) -> Result<(), CoreError> {
        // 没有失败记录时什么都不发布：runtime_change 无条件发新状态，会白白唤醒壳。
        if !lock(&self.state).failures.contains_key(&page) {
            return Ok(());
        }
        self.runtime_change(|state, _| {
            state.failures.remove(&page);
            Ok(())
        })
    }

    pub(super) fn dismiss_uninstalled(&self) -> Result<(), CoreError> {
        self.runtime_change(|state, _| {
            state.uninstalled.clear();
            Ok(())
        })
    }
}
