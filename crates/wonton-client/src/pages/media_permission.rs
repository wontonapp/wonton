//! 插件页在问摄像头/麦克风：按插件记决定，没问过的出询问，页面自己的源以外一律拒。

use url::Url;
use wonton_l10n::l10n;
use wonton_protocol::{CoreError, PanelScope, PlacedTab, TabRef};

use super::bridge::page_of;
use super::{PagesLink, lock, make_view, placed_tab_ref};
use crate::l10n::{CATALOG, language};

/// 核心对一次权限请求的处置。
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum TabMediaPermission {
    /// 直接放行；壳再去拿系统运行时权限。
    Allow,
    /// 直接拒绝；网页拿到拒绝，壳不出弹窗。
    Deny,
    /// 没问过：询问进了 [`PagesViewModel::media_prompt`](wonton_protocol::PagesViewModel)，
    /// 壳画成平台风格的弹窗，回答经 [`PagesLink::decide_media_permission`](PagesLink::decide_media_permission) 交回。
    Prompt,
}

/// 还没答的询问：挂在发起它的那一份页面上，页面关掉就清。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PendingMediaPrompt {
    pub(super) place: PlacedTab,
    pub(super) plugin_id: String,
    pub(super) camera: bool,
    pub(super) microphone: bool,
}

impl PagesLink {
    /// 壳转交的一次摄像头/麦克风请求（Android 的 `onPermissionRequest`、iOS 的媒体捕获权限代理）。
    /// `origin` 是发请求的网页源，从 WebView 的原生回调取，不从消息正文拿。页面关掉、
    /// 不是插件页或源对不上时直接拒；别的页面正等着回答时也直接拒，不让它干等。
    /// 同一页又来一份时，挂着的旧询问按“被新请求替换”判为拒绝：壳收到这次返回的 Prompt
    /// 就拒掉它手里旧的那份网页请求，询问窗按发出去的 ViewModel 换成新这份。
    pub(crate) fn request_media_permission(
        &self,
        page: &PlacedTab,
        origin: &str,
        camera: bool,
        microphone: bool,
    ) -> Result<TabMediaPermission, CoreError> {
        if !camera && !microphone {
            tracing::warn!(
                page = %page_id(page),
                category = "invalid_input",
                "the media request asks for neither camera nor microphone",
            );
            return Err(CoreError::InvalidInput {
                reason: l10n!(CATALOG, language(), "tabs-media-invalid-resources"),
            });
        }
        let Some((plugin_id, grant)) = self.media_decision(page, origin) else {
            tracing::info!(
                page = %page_id(page),
                category = "no_plugin_decision",
                "the media request is denied: the page is gone, not a plugin page, or the origin is not its own",
            );
            return Ok(TabMediaPermission::Deny);
        };
        match grant {
            Some(true) => {
                tracing::info!(
                    plugin_id = %plugin_id,
                    page = %page_id(page),
                    "the media request is allowed by the recorded decision",
                );
                return Ok(TabMediaPermission::Allow);
            }
            Some(false) => {
                tracing::info!(
                    plugin_id = %plugin_id,
                    page = %page_id(page),
                    category = "denied_by_decision",
                    "the media request is denied by the recorded decision",
                );
                return Ok(TabMediaPermission::Deny);
            }
            None => {}
        }
        let _writing = lock(&self.writing);
        let mut state = lock(&self.state).clone();
        if state
            .media_prompt
            .as_ref()
            .is_some_and(|pending| pending.place != *page)
        {
            tracing::info!(
                plugin_id = %plugin_id,
                page = %page_id(page),
                category = "another_page_pending",
                "the media request is denied: another page is waiting for its answer",
            );
            return Ok(TabMediaPermission::Deny);
        }
        // 走到这里只可能是同一页（别的页在等已经拒了）：挂着的旧询问在这里明确收场，
        // 不静默换人。旧的那份网页请求由壳处理：壳收到这次返回的 Prompt，拒掉它手里
        // 旧的那份，再按发出去的 ViewModel 给新这份出询问。
        if let Some(old) = state.media_prompt.take() {
            tracing::info!(
                plugin_id = %old.plugin_id,
                page = %page_id(page),
                category = "replaced_by_new_request",
                "the old media request is rejected: a new request from the same page replaces it",
            );
        }
        state.media_prompt = Some(PendingMediaPrompt {
            place: page.clone(),
            plugin_id: plugin_id.clone(),
            camera,
            microphone,
        });
        let (plugins, _, _, _) = self.inputs();
        let sync_stale = self.sync_problem.borrow().is_some();
        let view = make_view(&state, &plugins, sync_stale);
        *lock(&self.state) = state;
        self.publish(view, &plugins);
        tracing::info!(
            plugin_id = %plugin_id,
            page = %page_id(page),
            camera,
            microphone,
            "the media permission prompt is published; the answer comes back through decide_media_permission",
        );
        Ok(TabMediaPermission::Prompt)
    }

    /// 壳交回询问的回答：记下决定（存不下就不动状态，壳重试），并给出该给网页的结果。
    pub(crate) fn decide_media_permission(
        &self,
        allow: bool,
    ) -> Result<TabMediaPermission, CoreError> {
        let _writing = lock(&self.writing);
        let mut state = lock(&self.state).clone();
        let Some(pending) = state.media_prompt.take() else {
            tracing::warn!(
                category = "no_pending",
                "an answer arrived with no pending media prompt",
            );
            return Err(CoreError::InvalidInput {
                reason: l10n!(CATALOG, language(), "tabs-media-no-pending"),
            });
        };
        // 存不下就不动状态（take 只动了本地副本），壳重试时询问还在。
        self.plugins
            .record_media_decision(&pending.plugin_id, allow)
            .inspect_err(|error| {
                tracing::warn!(
                    plugin_id = %pending.plugin_id,
                    page = %page_id(&pending.place),
                    category = error_category(error),
                    %error,
                    "could not record the media decision; the prompt stays pending for a retry",
                )
            })?;
        let (plugins, _, _, _) = self.inputs();
        let sync_stale = self.sync_problem.borrow().is_some();
        let view = make_view(&state, &plugins, sync_stale);
        *lock(&self.state) = state;
        self.publish(view, &plugins);
        tracing::info!(
            plugin_id = %pending.plugin_id,
            page = %page_id(&pending.place),
            decision = if allow { "allow" } else { "deny" },
            "the media decision is recorded and the prompt cleared; the shell applies the answer to its web request",
        );
        Ok(if allow {
            TabMediaPermission::Allow
        } else {
            TabMediaPermission::Deny
        })
    }

    /// 发请求的页面认得出哪个插件、源对得上，就返回它的插件 ID 和记着的决定；
    /// 认不出（页面关了、不是插件页）或源是别的（页面里嵌的第三方 iframe）时是 `None`，一律拒。
    fn media_decision(&self, page: &PlacedTab, origin: &str) -> Option<(String, Option<bool>)> {
        let plugin_id = match placed_tab_ref(page) {
            TabRef::Plugin { plugin_id, .. }
            | TabRef::PluginPath { plugin_id, .. }
            | TabRef::Widget { plugin_id, .. }
            | TabRef::Bot {
                bot: wonton_protocol::BotRef { plugin_id, .. },
                ..
            } => plugin_id.clone(),
            // 公网网页、MCP Apps 宿主页和分享页不在这条路里：没有可记的插件决定。
            _ => return None,
        };
        let current = page_of(self, page).ok()?;
        let url = Url::parse(current.url.as_deref()?).ok()?;
        let origin = Url::parse(origin).ok()?;
        if !same_origin(&url, &origin) {
            return None;
        }
        let grant = self.plugins.media_grant(&plugin_id);
        Some((plugin_id, grant))
    }
}

/// 页面关掉了（栈和两层面板里都不在了）就清掉它的询问：壳的弹窗跟着消失，
/// 网页那头的请求由壳在销毁页面时拒绝。改动助手们都在 make_view 前调它。
pub(super) fn clear_gone_prompt(state: &mut super::PagesState) {
    if let Some(pending) = &state.media_prompt
        && !page_open(state, &pending.place)
    {
        state.media_prompt = None;
    }
}

fn page_open(state: &super::PagesState, place: &PlacedTab) -> bool {
    match place {
        PlacedTab::Page { tab } => state.stack.iter().any(|item| &item.tab == tab),
        PlacedTab::Panel { scope, tab } => match scope {
            PanelScope::Main => state.config.panel.iter().any(|entry| &entry.tab == tab),
            PanelScope::Room { room_id } => {
                state.open_room.as_ref().is_some_and(|(open, panel)| {
                    open == room_id && panel.pages.iter().any(|entry| &entry.tab == tab)
                })
            }
        },
    }
}

/// 同一个源：scheme、主机和有效端口都一样（桥的来源判定同一条规矩）。
pub(super) fn same_origin(url: &Url, origin: &Url) -> bool {
    origin.scheme() == url.scheme()
        && origin.host().is_some()
        && origin.host() == url.host()
        && origin.port_or_known_default() == url.port_or_known_default()
}

/// 日志里的页面标识：摆放位置加清单页 ID。不带完整路径和 URL——里面可能有主人浏览时
/// 留下的查询，日志不带用户内容。
fn page_id(page: &PlacedTab) -> String {
    let (placement, tab) = match page {
        PlacedTab::Page { tab } => ("page", tab),
        PlacedTab::Panel { scope, tab } => (
            match scope {
                PanelScope::Main => "panel/main",
                PanelScope::Room { .. } => "panel/room",
            },
            tab,
        ),
    };
    let name = match tab {
        TabRef::Plugin { page, .. } | TabRef::Bot { path: page, .. } => match page.as_deref() {
            Some(page) if !page.is_empty() => page,
            _ => "webui",
        },
        TabRef::PluginPath { .. } => "webui",
        TabRef::Url { .. } => "url",
        TabRef::UiResource { .. } => "mcp-apps",
        TabRef::SharedApp { .. } => "shared-app",
        // widget 页的房间 ID 不进日志，插件 ID 走单独的 plugin_id 字段。
        TabRef::Widget { .. } => "widget",
        TabRef::Chat => "chat",
        TabRef::Apps => "apps",
    };
    format!("{placement}/{name}")
}

/// 日志里的错误类别：按 [`CoreError`] 的变体名分因，走查时和壳的提示对得上，不混成一种。
fn error_category(error: &CoreError) -> &'static str {
    match error {
        CoreError::InvalidInput { .. } => "invalid_input",
        CoreError::Storage { .. } => "storage",
        CoreError::NotReady { .. } => "not_ready",
        CoreError::TransportUnavailable { .. } => "transport_unavailable",
        CoreError::Unauthorized { .. } => "unauthorized",
        CoreError::Forbidden { .. } => "forbidden",
        CoreError::Upstream { .. } => "upstream",
        CoreError::ServerTooOld { .. } => "server_too_old",
        CoreError::SettingsInvalid { .. } => "settings_invalid",
    }
}
