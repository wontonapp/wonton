//! 页面的动作：打开、关闭、我的应用的增删排序和自动加入。
//! 面板的动作在 [`panel`](super::panel)。
use super::*;

impl PagesLink {
    /// 打开一个页面：根页面一路弹回根，已经在栈里的挪到栈顶，其余压栈。
    pub(crate) fn open_page(&self, tab: TabRef) -> Result<(), CoreError> {
        self.runtime_change(|state, plugins| {
            if state.root_is(&tab) || tab == TabRef::Apps {
                state.stack.clear();
                return Ok(());
            }
            if !page_available(&tab, plugins) && !matches!(tab, TabRef::Plugin { .. }) {
                return Err(invalid(&l10n!(CATALOG, language(), "tabs-view-not-a-page")));
            }
            // 列表还没读到时，我的应用里已有的行照样打开：按存的元数据显示，状态页如实说加载中。
            let item = match stored_item_for_tab(&tab, plugins) {
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
        })
    }

    /// 关掉当前页。
    pub(super) fn close_page(&self) -> Result<(), CoreError> {
        self.runtime_change(|state, _| {
            if state.stack.pop().is_none() {
                return Err(invalid(&l10n!(
                    CATALOG,
                    language(),
                    "tabs-view-no-page-open"
                )));
            }
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
