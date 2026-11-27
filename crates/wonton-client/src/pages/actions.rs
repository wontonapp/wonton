//! 页面的动作：打开、关闭、我的应用的增删排序和自动加入。按网址加的在 [`add_url`](super::add_url)。
use super::*;

impl PagesLink {
    /// 打开一个页面：已经在栈里的挪到栈顶，其余压栈。根页（所有会话、收件箱）也走这里。
    pub(crate) fn open_page(&self, tab: TabRef) -> Result<(), CoreError> {
        if matches!(tab, TabRef::Widget { .. }) {
            return self.open_widget(tab);
        }
        self.runtime_change(|state, plugins| open_page_locked(state, plugins, tab))
    }

    /// 关掉当前页。没有开着的页时什么也不做：关页可以重复（连点两下关闭、关掉页面后紧跟一次返回），
    /// 不该因此弹出“没有打开的页面”。
    pub(super) fn close_page(&self) -> Result<(), CoreError> {
        self.runtime_change(|state, _| {
            state.stack.pop();
            Ok(())
        })
    }

    /// 点底栏的一项：两个根页切过去并弹回根；应用项打开它的整页（不带底栏）。
    pub(super) fn select_bar_item(&self, entry: BarEntry) -> Result<(), CoreError> {
        self.runtime_change(|state, plugins| {
            if !state.config.bar.contains(&entry) {
                return Err(invalid(&l10n!(CATALOG, language(), "tabs-bar-not-in-bar")));
            }
            match &entry {
                BarEntry::Agent | BarEntry::Apps => {
                    state.tab = entry;
                    state.stack.clear();
                    Ok(())
                }
                BarEntry::App { app } => {
                    let tab = openable_tab(app, plugins)?;
                    open_page_locked(state, plugins, tab)
                }
            }
        })
    }

    /// 把一行加到底栏末尾。满了或重复都如实拒，并记下“满了”给壳。
    pub(super) fn add_bar_item(&self, app: MyAppRef) -> Result<(), CoreError> {
        self.config_change(|state, _plugins| {
            let entry = BarEntry::App { app };
            if state.config.bar.contains(&entry) {
                return Err(invalid(&l10n!(
                    CATALOG,
                    language(),
                    "tabs-bar-already-pinned"
                )));
            }
            if state.config.bar.len() >= MAX_BAR_ITEMS as usize {
                state.bar_full = true;
                return Err(invalid(&l10n!(CATALOG, language(), "tabs-bar-full")));
            }
            state.config.bar.push(entry);
            state.bar_full = false;
            Ok(())
        })
    }

    /// 从底栏拿掉一项；最后一项不拿。
    pub(super) fn remove_bar_item(&self, entry: &BarEntry) -> Result<(), CoreError> {
        self.config_change(|state, _plugins| {
            let Some(index) = state.config.bar.iter().position(|item| item == entry) else {
                return Err(invalid(&l10n!(CATALOG, language(), "tabs-bar-not-in-bar")));
            };
            if state.config.bar.len() <= 1 {
                return Err(invalid(&l10n!(CATALOG, language(), "tabs-bar-empty")));
            }
            state.config.bar.remove(index);
            state.bar_full = false;
            state.settle_tab();
            Ok(())
        })
    }

    pub(super) fn move_bar_item(&self, from: u32, to: u32) -> Result<(), CoreError> {
        self.config_change(|state, _plugins| {
            let len = state.config.bar.len();
            let (from, to) = (from as usize, to as usize);
            if from >= len || to >= len {
                return Err(invalid(&l10n!(
                    CATALOG,
                    language(),
                    "tabs-bar-index-out-of-range"
                )));
            }
            let entry = state.config.bar.remove(from);
            state.config.bar.insert(to, entry);
            Ok(())
        })
    }

    pub(super) fn add_my_app(
        &self,
        app: MyAppRef,
        title: Option<String>,
        index: Option<u32>,
    ) -> Result<(), CoreError> {
        self.config_change(|state, plugins| {
            if state.config.position(&app).is_some() {
                return Err(invalid(&l10n!(
                    CATALOG,
                    language(),
                    "tabs-bar-already-pinned"
                )));
            }
            let entry = stored_app(&app, plugins, title)?;
            let index = index.map_or(state.config.my_apps.len(), |index| index as usize);
            if index > state.config.my_apps.len() {
                return Err(invalid(&l10n!(
                    CATALOG,
                    language(),
                    "tabs-bar-index-out-of-range"
                )));
            }
            state.config.my_apps.insert(index, entry);
            Ok(())
        })
    }

    pub(super) fn remove_my_app(&self, app: &MyAppRef) -> Result<(), CoreError> {
        self.config_change(|state, _| {
            let Some(index) = state.config.position(app) else {
                return Err(invalid(&l10n!(
                    CATALOG,
                    language(),
                    "tabs-bar-page-missing"
                )));
            };
            state.config.my_apps.remove(index);
            Ok(())
        })
    }

    pub(super) fn move_my_app(&self, app: &MyAppRef, index: u32) -> Result<(), CoreError> {
        self.config_change(|state, _| {
            let index = index as usize;
            if index >= state.config.my_apps.len() {
                return Err(invalid(&l10n!(
                    CATALOG,
                    language(),
                    "tabs-bar-index-out-of-range"
                )));
            }
            let Some(current) = state.config.position(app) else {
                return Err(invalid(&l10n!(
                    CATALOG,
                    language(),
                    "tabs-bar-page-missing"
                )));
            };
            let entry = state.config.my_apps.remove(current);
            state.config.my_apps.insert(index, entry);
            Ok(())
        })
    }

    pub(super) fn set_auto_add(&self, enabled: bool) -> Result<(), CoreError> {
        self.config_change(|state, plugins| {
            state.config.auto_add = enabled;
            if enabled {
                // 重新打开：现在装着的、还没加的应用都补进末尾（用户之后还能移除）。
                if let Some(installed) = &plugins.installed {
                    for plugin in &installed.plugins {
                        if plugin.install_failure.is_some()
                            || (plugin.web_url.is_none() && plugin.bot_user_id.is_none())
                        {
                            continue;
                        }
                        let app = MyAppRef::Plugin {
                            plugin_id: plugin.id.clone(),
                        };
                        if state.config.position(&app).is_none()
                            && let Ok(entry) = stored_app(&app, plugins, None)
                        {
                            state.config.my_apps.push(entry);
                        }
                    }
                }
            }
            Ok(())
        })
    }

    pub(super) fn reload(&self, page: PlacedTab) -> Result<(), CoreError> {
        if let PlacedTab::Page {
            tab: tab @ TabRef::Widget { .. },
        } = &page
        {
            return self.reopen_widget(tab.clone());
        }
        self.runtime_change(|state, _| {
            if !is_web_page(placed_tab_ref(&page)) || !state_has_page(state, &page) {
                return Err(invalid(&l10n!(
                    CATALOG,
                    language(),
                    "tabs-page-cannot-reload"
                )));
            }
            let page = canonical_page(page);
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
            } else if !plugin_is_running(state, tab, plugins) {
                TabLoadFailure::PluginNotReady
            } else {
                TabLoadFailure::Upstream
            };
            state
                .failures
                .insert(canonical_page(page), (failure, status));
            Ok(())
        })
    }

    pub(crate) fn load_succeeded(&self, page: PlacedTab) -> Result<(), CoreError> {
        {
            let state = lock(&self.state);
            if !state_has_page(&state, &page) {
                return Err(invalid(&l10n!(
                    CATALOG,
                    language(),
                    "tabs-bridge-page-not-open"
                )));
            }
            if !state.failures.contains_key(&canonical_page(page.clone())) {
                return Ok(());
            }
        }
        self.runtime_change(|state, _| {
            if !state_has_page(state, &page) {
                return Err(invalid(&l10n!(
                    CATALOG,
                    language(),
                    "tabs-bridge-page-not-open"
                )));
            }
            state.failures.remove(&canonical_page(page));
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

/// 打开一页（调用方已经拿着状态锁）：根页直接压栈，其余按插件和缓存解析。
fn open_page_locked(
    state: &mut PagesState,
    plugins: &PluginsViewModel,
    tab: TabRef,
) -> Result<(), CoreError> {
    if matches!(tab, TabRef::Rooms | TabRef::Inbox) {
        if state.stack.last().is_some_and(|open| open.tab == tab) {
            return Ok(());
        }
        state.stack.retain(|open| open.tab != tab);
        state.stack.push(root_page_item(&tab));
        return Ok(());
    }
    if !page_available(&tab, plugins) && !matches!(tab, TabRef::Plugin { .. }) {
        return Err(invalid(&l10n!(CATALOG, language(), "tabs-view-not-a-page")));
    }
    // 列表还没读到时，我的应用里已有的行照样打开：按存的元数据显示，状态页如实说加载中。
    let item = match bot::stored_item_or_plugin(state, &tab, plugins, None) {
        Ok(item) => item,
        Err(error @ CoreError::NotReady { .. }) => {
            stored_item_from_config(&state.config, &tab).ok_or(error)?
        }
        Err(error) => return Err(error),
    };
    if state.stack.last().is_some_and(|open| open.tab == tab) {
        return Ok(());
    }
    state.stack.retain(|open| open.tab != tab);
    state.stack.push(item);
    Ok(())
}

/// 根页压进栈时的元数据；壳按页面身份画自己的原生列表，标题和图标只给胶囊用。
fn root_page_item(tab: &TabRef) -> StoredItem {
    let (title, plugin_title) = match tab {
        TabRef::Inbox => (
            l10n!(CATALOG, language(), "tabs-inbox-title"),
            "Wonton".to_owned(),
        ),
        _ => (
            l10n!(CATALOG, language(), "tabs-rooms-title"),
            "Wonton".to_owned(),
        ),
    };
    StoredItem {
        tab: tab.clone(),
        page: Some(StoredPage {
            plugin_title,
            title,
            icon: TabIcon::Plugin { icon: None },
            layout: TabLayout::Inset,
        }),
    }
}

/// 底栏应用项打开哪一页：页面行按它自己，插件行开它的第一页。
fn openable_tab(app: &MyAppRef, plugins: &PluginsViewModel) -> Result<TabRef, CoreError> {
    match app {
        MyAppRef::Page { tab } => Ok(tab.clone()),
        MyAppRef::Plugin { plugin_id } => {
            let plugin = plugins
                .installed
                .as_ref()
                .and_then(|list| list.plugins.iter().find(|plugin| plugin.id == *plugin_id))
                .ok_or_else(|| invalid(&l10n!(CATALOG, language(), "tabs-view-unknown-plugin")))?;
            let page = plugin.tabs.first().map(|page| page.id.clone());
            Ok(TabRef::Plugin {
                plugin_id: plugin_id.clone(),
                page,
            })
        }
    }
}
