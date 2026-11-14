//! 聊天页面板(ADR-0052):页面列表在账号数据里,动作、校验和房间面板的读写都在这儿。
//!
//! 面板分主面板(全局账号数据 `app.wonton.layout`)和每个房间的面板(房间账号数据
//! `app.wonton.room_panel`)。最多 5 页;能放插件页、插件里的某一页和公网网页(只收 https)。
//! 错误给类别,文字在客户端的 .ftl。
use super::*;

impl PagesLink {
    /// 把页面放进面板的最后。`title` 是用户起的名字（钉当前页时带上），候选页来的没有。
    pub(super) fn add_panel_page(
        &self,
        scope: PanelScope,
        tab: TabRef,
        title: Option<String>,
    ) -> Result<(), CoreError> {
        let plugins = self.plugins_view.borrow().clone();
        {
            let state = lock(&self.state);
            let entries = panel_entries(&state, &scope)?;
            if entries.iter().any(|entry| entry.tab == tab) {
                return Err(invalid(&l10n!(
                    CATALOG,
                    language(),
                    "panel-page-already-there"
                )));
            }
            let bot_pages = state
                .bot_pins
                .iter()
                .filter(|pin| pin.scope == scope)
                .filter(|pin| !entries.iter().any(|entry| entry.tab == pin.tab))
                .count();
            if entries.len() + bot_pages >= MAX_PANEL_PAGES {
                return Err(invalid(&l10n!(CATALOG, language(), "panel-full")));
            }
        }
        check_placeable(&tab, &plugins)?;
        // 名字落进条目：卸载插件时报告才知道拿掉的是哪些页，视图也不用每次再算。
        let title = Some(title.unwrap_or_else(|| default_panel_title(&tab, &plugins)));
        match scope {
            PanelScope::Main => self.config_change(|state, _| {
                state.config.panel.push(PanelEntry { tab, title });
                Ok(())
            }),
            PanelScope::Room { room_id } => self.write_room_panel(&room_id, |panel| {
                panel.pages.push(PanelEntry { tab, title });
            }),
        }
    }

    pub(super) fn remove_panel_page(
        &self,
        scope: PanelScope,
        tab: TabRef,
    ) -> Result<(), CoreError> {
        // Bot 钉的页在服务器上：没有用户自己放的同名页时，经接口拿掉。
        let bot_pin = {
            let state = lock(&self.state);
            let user_has = panel_entries(&state, &scope)
                .is_ok_and(|entries| entries.iter().any(|entry| entry.tab == tab));
            (!user_has)
                .then(|| {
                    state
                        .bot_pins
                        .iter()
                        .find(|pin| pin.scope == scope && pin.tab == tab)
                        .cloned()
                })
                .flatten()
        };
        if let Some(pin) = bot_pin {
            let weak = lock(&self.weak).clone();
            self.runtime.spawn(async move {
                let Some(link) = weak.upgrade() else { return };
                match link.remove_bot_pin(&pin.id).await {
                    Ok(()) => {
                        let mut state = lock(&link.state);
                        state.bot_pins.retain(|candidate| candidate.id != pin.id);
                        drop(state);
                        link.recompute();
                    }
                    Err(error) => {
                        let message = error.to_string();
                        link.runtime_change(|state, _| {
                            state.problem = Some(PagesProblem { message });
                            Ok(())
                        })
                        .ok();
                    }
                }
            });
            return Ok(());
        }
        match &scope {
            PanelScope::Main => self.config_change(|state, _| {
                let index = state
                    .config
                    .panel
                    .iter()
                    .position(|entry| entry.tab == tab)
                    .ok_or_else(not_in_panel)?;
                state.config.panel.remove(index);
                state.attention.remove(&PlacedTab::Panel {
                    scope: PanelScope::Main,
                    tab: tab.clone(),
                });
                if state.panel_selected.as_ref() == Some(&tab) {
                    state.panel_selected = None;
                }
                Ok(())
            }),
            PanelScope::Room { room_id } => {
                let room_id = room_id.clone();
                {
                    let state = lock(&self.state);
                    panel_entries(&state, &scope)?
                        .iter()
                        .find(|entry| entry.tab == tab)
                        .ok_or_else(not_in_panel)?;
                }
                self.write_room_panel(&room_id, move |panel| {
                    panel.pages.retain(|entry| entry.tab != tab);
                })
            }
        }
    }

    pub(super) fn move_panel_page(
        &self,
        scope: PanelScope,
        tab: TabRef,
        index: u32,
    ) -> Result<(), CoreError> {
        match &scope {
            PanelScope::Main => {
                let fixed = {
                    let state = lock(&self.state);
                    !state.config.panel.iter().any(|entry| entry.tab == tab)
                        && state
                            .bot_pins
                            .iter()
                            .any(|pin| pin.scope == scope && pin.tab == tab)
                };
                if fixed {
                    return Ok(());
                }
                self.config_change(|state, _| move_entry(&mut state.config.panel, &tab, index))
            }
            PanelScope::Room { room_id } => {
                let room_id = room_id.clone();
                {
                    let state = lock(&self.state);
                    let entries = panel_entries(&state, &scope)?;
                    if !entries.iter().any(|entry| entry.tab == tab) {
                        // Bot 钉的页按钉的先后固定，拖动当成功但不动。
                        if state
                            .bot_pins
                            .iter()
                            .any(|pin| pin.scope == scope && pin.tab == tab)
                        {
                            return Ok(());
                        }
                        return Err(not_in_panel());
                    }
                    if index as usize >= entries.len() {
                        return Err(invalid(&l10n!(
                            CATALOG,
                            language(),
                            "panel-index-out-of-range"
                        )));
                    }
                }
                self.write_room_panel(&room_id, move |panel| {
                    if let Some(current) = panel.pages.iter().position(|entry| entry.tab == tab) {
                        let entry = panel.pages.remove(current);
                        let at = (index as usize).min(panel.pages.len());
                        panel.pages.insert(at, entry);
                    }
                })
            }
        }
    }

    /// 记下面板在看哪一页（本机，下次拉开还是它），并清掉这一页的红点。
    pub(super) fn select_panel_page(
        &self,
        scope: PanelScope,
        tab: TabRef,
    ) -> Result<(), CoreError> {
        let seen_tab = tab.clone();
        self.runtime_change(|state, _| {
            let placed = PlacedTab::Panel {
                scope: scope.clone(),
                tab: tab.clone(),
            };
            state.attention.remove(&placed);
            if let Some(pin) = state
                .bot_pins
                .iter_mut()
                .find(|pin| pin.scope == scope && pin.tab == tab)
            {
                pin.attention = false;
            }
            match scope {
                PanelScope::Main => state.panel_selected = Some(tab),
                PanelScope::Room { ref room_id } => {
                    let room = state.rooms.get_mut(room_id).ok_or_else(not_in_panel)?;
                    room.selected = Some(tab);
                }
            }
            Ok(())
        })?;
        let seen = {
            let state = lock(&self.state);
            state
                .bot_pins
                .iter()
                .find(|pin| pin.scope == scope && pin.tab == seen_tab)
                .map(|pin| pin.id.clone())
        };
        if let Some(id) = seen {
            let weak = lock(&self.weak).clone();
            self.runtime.spawn(async move {
                if let Some(link) = weak.upgrade() {
                    let _ = link.seen_bot_pin(&id).await;
                }
            });
        }
        Ok(())
    }

    /// 把正在看的页面钉到面板或我的应用：按网址算出是哪一种页面。
    pub(super) fn pin_current_page(
        &self,
        url: &str,
        title: Option<String>,
        to: PinTarget,
    ) -> Result<(), CoreError> {
        let plugins = self.plugins_view.borrow().clone();
        let (tab, default_title) = classify_pin(url, &plugins)?;
        let title = title.or(default_title);
        match to {
            PinTarget::Panel { scope } => self.add_panel_page(scope, tab, title),
            PinTarget::MyApps => {
                let app = match &tab {
                    TabRef::Plugin { plugin_id, page } if page.is_none() => MyAppRef::Plugin {
                        plugin_id: plugin_id.clone(),
                    },
                    _ => MyAppRef::Page { tab },
                };
                self.add_my_app(app, title, None)
            }
        }
    }

    /// 壳报告房间里盖着消息列表的东西开没开。不是打开着的房间就忽略（壳的报告迟到了）。
    pub(super) fn set_room_entry_blocked(
        &self,
        room_id: &str,
        blocked: bool,
    ) -> Result<(), CoreError> {
        self.runtime_change(|state, _| {
            if let Some(room) = state.rooms.get_mut(room_id) {
                room.blocked = blocked;
            }
            Ok(())
        })
    }

    /// 房间面板的改动：写进房间账号数据，成功才算改了（房间面板没有本机文件，离线开的是
    /// 同步到的旧内容）。改动是否合法由调用方先对着当前内容验好；这里只负责写和喂回。
    /// 成功后把新内容喂回去，视图马上跟上；失败显示原因，面板不动。
    pub(super) fn write_room_panel(
        &self,
        room_id: &str,
        change: impl FnOnce(&mut RoomPanel) + Send + 'static,
    ) -> Result<(), CoreError> {
        self.assert_room_open(room_id)?;
        let session = self
            .session
            .borrow()
            .clone()
            .ok_or_else(|| CoreError::NotReady {
                reason: l10n!(CATALOG, language(), "panel-not-signed-in"),
            })?;
        let Ok(room_id) = room_id.parse::<OwnedRoomId>() else {
            return Err(invalid(&l10n!(CATALOG, language(), "panel-room-unknown")));
        };
        let weak = lock(&self.weak).clone();
        let sync_problem = self.sync_problem.clone();
        self.runtime.spawn(async move {
            let written = session
                .update_room_panel(&room_id, |raw| {
                    let mut panel: RoomPanel =
                        serde_json::from_value(std::mem::take(raw).into()).unwrap_or_default();
                    change(&mut panel);
                    *raw = serde_json::to_value(panel)
                        .expect("room panel only has serializable fields")
                        .as_object()
                        .cloned()
                        .unwrap_or_default();
                })
                .await;
            match written {
                Ok(content) => {
                    sync_problem.send_replace(None);
                    if let Ok(panel) = serde_json::from_value::<RoomPanel>(content.0.into())
                        && let Some(link) = weak.upgrade()
                    {
                        let _ = link.receive_room_panel(room_id, panel);
                    }
                }
                Err(error) => {
                    sync_problem.send_replace(Some(error.to_string()));
                }
            }
        });
        Ok(())
    }

    pub(super) fn assert_room_open(&self, room_id: &str) -> Result<(), CoreError> {
        let state = lock(&self.state);
        if state.rooms.contains_key(room_id) {
            return Ok(());
        }
        Err(invalid(&l10n!(CATALOG, language(), "panel-room-not-open")))
    }

    /// 页面要求关注（红点）。只有面板里的实例有红点可挂；翻到那页时壳发 SelectPanelPage 清掉。
    pub(crate) fn set_attention(
        &self,
        placed: PlacedTab,
        attention: bool,
    ) -> Result<(), CoreError> {
        if !matches!(placed, PlacedTab::Panel { .. }) {
            return Err(invalid(&l10n!(
                CATALOG,
                language(),
                "panel-attention-panel-only"
            )));
        }
        self.runtime_change(|state, _| {
            if !state_has_page(state, &placed) {
                return Err(invalid(&l10n!(
                    CATALOG,
                    language(),
                    "tabs-bridge-page-not-open"
                )));
            }
            if attention {
                state.attention.insert(placed);
            } else {
                state.attention.remove(&placed);
            }
            Ok(())
        })
    }
}

fn panel_entries<'a>(
    state: &'a PagesState,
    scope: &PanelScope,
) -> Result<&'a Vec<PanelEntry>, CoreError> {
    match scope {
        PanelScope::Main => Ok(&state.config.panel),
        PanelScope::Room { room_id } => state
            .rooms
            .get(room_id)
            .map(|room| &room.panel.pages)
            .ok_or_else(not_in_panel),
    }
}

fn not_in_panel() -> CoreError {
    invalid(&l10n!(CATALOG, language(), "panel-page-not-there"))
}

/// 只有插件页、插件里的某一页和公网网页能放进面板；插件不在了（或还没装好列表）放不了。
fn check_placeable(tab: &TabRef, plugins: &PluginsViewModel) -> Result<(), CoreError> {
    match tab {
        TabRef::Plugin { .. } | TabRef::PluginPath { .. } => {
            let Some(installed) = plugins.installed.as_ref() else {
                return Err(CoreError::NotReady {
                    reason: l10n!(CATALOG, language(), "tabs-view-plugin-list-not-ready"),
                });
            };
            let id = match tab {
                TabRef::Plugin { plugin_id, .. } | TabRef::PluginPath { plugin_id, .. } => {
                    plugin_id
                }
                _ => unreachable!("matched above"),
            };
            let Some(plugin) = installed.plugins.iter().find(|plugin| plugin.id == *id) else {
                return Err(invalid(&l10n!(
                    CATALOG,
                    language(),
                    "tabs-view-unknown-plugin"
                )));
            };
            checked_plugin_path(tab, plugin).map(|_| ())
        }
        TabRef::Url { url } => {
            if is_https_url(url) {
                return Ok(());
            }
            Err(invalid(&l10n!(CATALOG, language(), "panel-pin-not-https")))
        }
        _ => Err(invalid(&l10n!(
            CATALOG,
            language(),
            "panel-page-unsupported"
        ))),
    }
}

/// 没给名字时的默认标题：插件页用清单标题，网址用域名。
fn default_panel_title(tab: &TabRef, plugins: &PluginsViewModel) -> String {
    match tab {
        TabRef::Plugin { plugin_id, .. } | TabRef::PluginPath { plugin_id, .. } => plugins
            .installed
            .as_ref()
            .and_then(|list| list.plugins.iter().find(|plugin| plugin.id == *plugin_id))
            .map(|plugin| plugin.title.clone())
            .unwrap_or_default(),
        TabRef::Url { url } => Url::parse(url)
            .ok()
            .and_then(|parsed| parsed.host_str().map(str::to_owned))
            .unwrap_or_else(|| url.clone()),
        _ => String::new(),
    }
}

fn move_entry(entries: &mut Vec<PanelEntry>, tab: &TabRef, index: u32) -> Result<(), CoreError> {
    let index = index as usize;
    if index >= entries.len() {
        return Err(invalid(&l10n!(
            CATALOG,
            language(),
            "panel-index-out-of-range"
        )));
    }
    let Some(current) = entries.iter().position(|entry| entry.tab == *tab) else {
        return Err(not_in_panel());
    };
    let entry = entries.remove(current);
    entries.insert(index, entry);
    Ok(())
}

/// 正在看的网址是哪一种页面：装着的插件的网站里就是那个插件的页（路径对上清单就是声明的那页，
/// 否则是插件里的某一页）；不是插件的网站只收 `https://`。返回页面身份和没有用户名字时的默认标题。
fn classify_pin(
    url: &str,
    plugins: &PluginsViewModel,
) -> Result<(TabRef, Option<String>), CoreError> {
    let parsed = Url::parse(url)
        .ok()
        .filter(|url| !url.cannot_be_a_base())
        .ok_or_else(|| invalid(&l10n!(CATALOG, language(), "panel-pin-invalid-url")))?;
    if let Some(installed) = &plugins.installed {
        for plugin in &installed.plugins {
            let Some(base) = plugin
                .web_url
                .as_deref()
                .and_then(|base| Url::parse(base).ok())
            else {
                continue;
            };
            if base.scheme() != parsed.scheme()
                || base.host_str() != parsed.host_str()
                || base.port_or_known_default() != parsed.port_or_known_default()
            {
                continue;
            }
            let mut path = parsed.path().to_owned();
            if let Some(query) = parsed.query() {
                path.push('?');
                path.push_str(query);
            }
            if let Some(fragment) = parsed.fragment() {
                path.push('#');
                path.push_str(fragment);
            }
            if path == "/" {
                return Ok((
                    TabRef::Plugin {
                        plugin_id: plugin.id.clone(),
                        page: None,
                    },
                    Some(plugin.title.clone()),
                ));
            }
            if let Some(declared) = plugin.tabs.iter().find(|tab| tab.path == path) {
                return Ok((
                    TabRef::Plugin {
                        plugin_id: plugin.id.clone(),
                        page: Some(declared.id.clone()),
                    },
                    Some(declared.title.clone()),
                ));
            }
            return Ok((
                TabRef::PluginPath {
                    plugin_id: plugin.id.clone(),
                    path,
                },
                Some(plugin.title.clone()),
            ));
        }
    }
    if parsed.scheme() != "https" || parsed.host_str().is_none() {
        return Err(invalid(&l10n!(CATALOG, language(), "panel-pin-not-https")));
    }
    Ok((
        TabRef::Url {
            url: parsed.to_string(),
        },
        parsed.host_str().map(str::to_owned),
    ))
}
