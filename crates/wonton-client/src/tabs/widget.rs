//! 插件网页作为 Matrix widget 在房间里打开（ADR-0096）：选中 [`TabRef::Widget`] 时起 matrix-sdk 的驱动，
//! 页面关掉、换到别的房间、退出登录时结束；同一时间只有一个。页面和驱动之间的消息经桥来回
//! （[`super::bridge`]），驱动发给页面的由壳订阅（[`TabWidgetSubscription`]）。

use std::sync::atomic::AtomicU64;

use tokio::task::JoinHandle;
use wonton_matrix::{OpenedWidget, WidgetDriverHandle, WidgetRequest};
use wonton_protocol::{InstalledPlugin, RoomWidget, TabLayout, TabPage, TabState};

use super::view::{page_from_metadata, page_state, page_url};
use super::*;

/// 清单 `matrix_widget.url` 里核心替换的占位符：页面自己的 origin（Element Call 的 `parentUrl` 要它）。
const PAGE_ORIGIN: &str = "$org.wonton.page_origin";

/// 打开着的 widget 页：地址出来之前是加载中，起不来是出错。
#[derive(Clone)]
pub(super) struct WidgetPage {
    pub(super) tab: TabRef,
    url: Option<String>,
    problem: Option<String>,
    /// 第几次打开：起驱动是异步的，对不上号的晚到结果丢掉。
    generation: u64,
}

/// 跑着的驱动。丢掉它（页面关了、换了一个）驱动就停，页面那头的订阅随之结束。
pub(super) struct Session {
    tab: TabRef,
    generation: u64,
    handle: Arc<WidgetDriverHandle>,
    task: JoinHandle<()>,
}

impl Drop for Session {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// [`TabsLink`] 里 widget 用的两样：跑着的驱动和打开的次数。
#[derive(Default)]
pub(super) struct Sessions {
    current: Mutex<Option<Session>>,
    opened: AtomicU64,
}

impl TabsLink {
    /// 选中一个 widget 页：盖在聊天上全屏打开，起一个新的驱动。已经开着的同一页不重来。
    pub(super) fn open_widget(&self, tab: TabRef) -> Result<(), CoreError> {
        let generation = self.widgets.opened.fetch_add(1, Ordering::SeqCst) + 1;
        let mut fresh = false;
        self.runtime_change(|state, plugins| {
            let shown = state.overlay.as_ref().is_some_and(|item| item.tab == tab);
            if shown && state.widget.as_ref().is_some_and(|page| page.tab == tab) {
                return Ok(());
            }
            let item = item(&tab, plugins)?;
            let previous = state.selected.clone();
            state.overlay = Some(item);
            state.fullscreen = Some(Fullscreen {
                tab: tab.clone(),
                return_to: Some(previous),
            });
            state.widget = Some(WidgetPage {
                tab: tab.clone(),
                url: None,
                problem: None,
                generation,
            });
            fresh = true;
            Ok(())
        })?;
        if fresh {
            self.start_widget(tab, generation);
        }
        Ok(())
    }

    /// 重新加载 widget 页：同一个驱动接不住页面再握一次手，换一个新的（新的 widget ID，地址跟着变）。
    pub(super) fn reopen_widget(&self, tab: TabRef) -> Result<(), CoreError> {
        let generation = self.widgets.opened.fetch_add(1, Ordering::SeqCst) + 1;
        self.runtime_change(|state, _| {
            let shown = state.overlay.as_ref().is_some_and(|item| item.tab == tab);
            if !shown {
                return Err(invalid(&l10n!(
                    CATALOG,
                    language(),
                    "tabs-page-cannot-reload"
                )));
            }
            state.widget = Some(WidgetPage {
                tab: tab.clone(),
                url: None,
                problem: None,
                generation,
            });
            Ok(())
        })?;
        self.start_widget(tab, generation);
        Ok(())
    }

    fn start_widget(&self, tab: TabRef, generation: u64) {
        let session = self.session.borrow().clone();
        let request = request(&tab, &self.plugins_view.borrow());
        let weak = lock(&self.weak).clone();
        self.runtime.spawn(async move {
            let opened = match (session, request) {
                (None, _) => Err(l10n!(CATALOG, language(), "tabs-widget-not-signed-in")),
                (_, Err(error)) => Err(error.to_string()),
                (Some(session), Ok((room_id, request))) => session
                    .open_widget(&room_id, request)
                    .await
                    .map_err(|error| error.to_string()),
            };
            if let Some(link) = weak.upgrade() {
                link.widget_started(generation, opened);
            }
        });
    }

    fn widget_started(&self, generation: u64, opened: Result<OpenedWidget, String>) {
        let current = |state: &TabsState| {
            state
                .widget
                .as_ref()
                .is_some_and(|page| page.generation == generation)
        };
        let opened = match opened {
            Ok(opened) => opened,
            Err(problem) => {
                let _ = self.runtime_change(|state, _| {
                    if current(state)
                        && let Some(page) = &mut state.widget
                    {
                        page.problem = Some(problem);
                    }
                    Ok(())
                });
                return;
            }
        };
        let Some(tab) = lock(&self.state)
            .widget
            .as_ref()
            .filter(|page| page.generation == generation)
            .map(|page| page.tab.clone())
        else {
            return;
        };
        let OpenedWidget {
            url,
            handle,
            driver,
        } = opened;
        // 换了会话（退出登录、断线重登）就停：驱动拿着的房间属于旧的 Matrix 客户端。
        let mut session = self.session.clone();
        // 克隆来的接收端带着原来的“已读”位置；`TabsLink` 只 borrow 不标已读，不标的话 `changed()` 当场返回。
        session.mark_unchanged();
        let weak = lock(&self.weak).clone();
        let task = self.runtime.spawn(async move {
            tokio::select! {
                () = driver => {}
                _ = session.changed() => {}
            }
            if let Some(link) = weak.upgrade() {
                link.widget_ended(generation);
            }
        });
        *lock(&self.widgets.current) = Some(Session {
            tab,
            generation,
            handle: Arc::new(handle),
            task,
        });
        let _ = self.runtime_change(|state, _| {
            if current(state)
                && let Some(page) = &mut state.widget
            {
                page.url = Some(url);
            }
            Ok(())
        });
    }

    /// 驱动自己停了：页面跟着关掉，不留一个说不上话的页面。
    fn widget_ended(&self, generation: u64) {
        let _ = self.runtime_change(|state, _| {
            let Some(page) = state.widget.take_if(|page| page.generation == generation) else {
                return Ok(());
            };
            if state
                .overlay
                .as_ref()
                .is_some_and(|item| item.tab == page.tab)
            {
                state.overlay = None;
                state.fullscreen = None;
            }
            Ok(())
        });
    }

    /// 发布视图时对一下账：页面已经不是这一次打开的了，驱动就停。
    pub(super) fn settle_widget(&self, open: Option<u64>) {
        let mut current = lock(&self.widgets.current);
        if current
            .as_ref()
            .is_some_and(|session| Some(session.generation) != open)
        {
            *current = None;
        }
    }

    /// 页面发来的 widget 消息交给驱动。不是跑着驱动的那一页时丢掉。
    pub(crate) fn send_to_widget(&self, tab: &TabRef, message: &str) {
        if let Some(handle) = self.widget_handle(tab) {
            handle.send(message.to_owned());
        }
    }

    pub(crate) fn widget_handle(&self, tab: &TabRef) -> Option<Arc<WidgetDriverHandle>> {
        lock(&self.widgets.current)
            .as_ref()
            .filter(|session| session.tab == *tab)
            .map(|session| session.handle.clone())
    }
}

/// 页面不显示了、或者聊天换到了别的房间，widget 页就算关了；后者连覆盖层一起收掉。
pub(super) fn clear_closed(state: &mut TabsState) {
    let Some(page) = &state.widget else {
        return;
    };
    let shown = state
        .overlay
        .as_ref()
        .is_some_and(|item| item.tab == page.tab);
    let elsewhere = match (&page.tab, &state.open_room) {
        (TabRef::Widget { room_id, .. }, Some((open, _))) => open.as_str() != room_id,
        _ => false,
    };
    if shown && elsewhere {
        state.overlay = None;
        state.fullscreen = None;
    }
    if !shown || elsewhere {
        state.widget = None;
    }
}

pub(super) fn open_generation(state: &TabsState) -> Option<u64> {
    state.widget.as_ref().map(|page| page.generation)
}

fn plugin<'a>(
    plugin_id: &str,
    plugins: &'a PluginsViewModel,
) -> Result<&'a InstalledPlugin, CoreError> {
    let installed = plugins
        .installed
        .as_ref()
        .ok_or_else(|| CoreError::NotReady {
            reason: l10n!(CATALOG, language(), "tabs-view-plugin-list-not-ready"),
        })?;
    installed
        .plugins
        .iter()
        .find(|plugin| plugin.id == plugin_id)
        .ok_or_else(|| invalid(&l10n!(CATALOG, language(), "tabs-view-unknown-plugin")))
}

/// 覆盖层上的那一项。插件得声明了 `matrix_widget`，房间 ID 得像样。
fn item(tab: &TabRef, plugins: &PluginsViewModel) -> Result<StoredItem, CoreError> {
    let TabRef::Widget { plugin_id, room_id } = tab else {
        return Err(invalid(&l10n!(
            CATALOG,
            language(),
            "tabs-widget-not-declared"
        )));
    };
    OwnedRoomId::try_from(room_id.as_str()).map_err(|error| invalid(&error.to_string()))?;
    let plugin = plugin(plugin_id, plugins)?;
    if plugin.matrix_widget.is_none() || plugin.web_url.is_none() {
        return Err(invalid(&l10n!(
            CATALOG,
            language(),
            "tabs-widget-not-declared"
        )));
    }
    Ok(StoredItem {
        tab: tab.clone(),
        page: Some(StoredPage {
            plugin_title: plugin.title.clone(),
            title: plugin.title.clone(),
            icon: TabIcon::Plugin {
                icon: plugin.icon.clone(),
            },
            layout: TabLayout::EdgeToEdge,
        }),
    })
}

/// 起驱动要的房间和地址：清单的路径接在插件网页的 origin 后面，`$org.wonton.page_origin` 换成这个 origin。
fn request(
    tab: &TabRef,
    plugins: &PluginsViewModel,
) -> Result<(OwnedRoomId, WidgetRequest), CoreError> {
    let TabRef::Widget { plugin_id, room_id } = tab else {
        return Err(invalid(&l10n!(
            CATALOG,
            language(),
            "tabs-widget-not-declared"
        )));
    };
    let room_id =
        OwnedRoomId::try_from(room_id.as_str()).map_err(|error| invalid(&error.to_string()))?;
    let plugin = plugin(plugin_id, plugins)?;
    let not_declared = || invalid(&l10n!(CATALOG, language(), "tabs-widget-not-declared"));
    let widget = plugin.matrix_widget.as_ref().ok_or_else(not_declared)?;
    let url = page_url(plugin.web_url.as_deref(), Some(&widget.url)).ok_or_else(not_declared)?;
    let origin = Url::parse(&url)
        .map_err(|_| not_declared())?
        .origin()
        .ascii_serialization();
    let origin: String = url::form_urlencoded::byte_serialize(origin.as_bytes()).collect();
    Ok((
        room_id,
        WidgetRequest {
            url: url.replace(PAGE_ORIGIN, &origin),
            declared: widget.capabilities.clone(),
            language: CATALOG.tag(language()).to_owned(),
        },
    ))
}

/// 覆盖层上 widget 页的样子：地址出来前是加载中，驱动起不来是出错（原因是详情）。
pub(super) fn page(
    state: &TabsState,
    placed: &PlacedTab,
    stored: &StoredItem,
    plugins: &PluginsViewModel,
) -> TabPage {
    let TabRef::Widget { plugin_id, .. } = &stored.tab else {
        unreachable!("widget::page is only built for TabRef::Widget");
    };
    let plugin = plugin(plugin_id, plugins).ok();
    let current = state.widget.as_ref().filter(|page| page.tab == stored.tab);
    let url = current.and_then(|page| page.url.clone());
    let state_value = match (plugin, current.and_then(|page| page.problem.clone())) {
        (None, _) => TabState::PageRemoved,
        (Some(_), Some(reason)) => TabState::Failed { reason },
        (Some(plugin), None) => match page_state(state, placed, plugin, true) {
            TabState::Ready if url.is_none() => TabState::Loading,
            other => other,
        },
    };
    let metadata = stored.page.clone().unwrap_or_else(|| StoredPage {
        plugin_title: plugin_id.clone(),
        title: plugin_id.clone(),
        icon: TabIcon::Plugin { icon: None },
        layout: TabLayout::EdgeToEdge,
    });
    page_from_metadata(
        state,
        placed,
        metadata,
        url,
        state_value,
        TabDisplayChoice::Fullscreen,
        plugins,
    )
}

/// 能打开 widget 页的插件：声明了 `matrix_widget`、有网页、在运行（ADR-0096 的房间顶栏入口）。
pub(super) fn available(tab: &TabRef, plugins: &PluginsViewModel) -> bool {
    let TabRef::Widget { plugin_id, .. } = tab else {
        return false;
    };
    plugin(plugin_id, plugins)
        .is_ok_and(|plugin| plugin.matrix_widget.is_some() && plugin.web_url.is_some())
}

/// 打开的房间顶栏上的入口，按已装列表的顺序。
pub(super) fn entries(room_id: &str, plugins: &PluginsViewModel) -> Vec<RoomWidget> {
    let Some(installed) = &plugins.installed else {
        return Vec::new();
    };
    installed
        .plugins
        .iter()
        .filter(|plugin| {
            plugin.web_url.is_some() && matches!(plugin.status, PluginState::Running { .. })
        })
        .filter_map(|plugin| {
            let widget = plugin.matrix_widget.as_ref()?;
            Some(RoomWidget {
                tab: TabRef::Widget {
                    plugin_id: plugin.id.clone(),
                    room_id: room_id.to_owned(),
                },
                title: plugin.title.clone(),
                icon: plugin.icon.clone(),
                entry: widget.entry,
            })
        })
        .collect()
}
