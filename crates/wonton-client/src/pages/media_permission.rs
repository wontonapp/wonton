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
    pub(crate) fn request_media_permission(
        &self,
        page: &PlacedTab,
        origin: &str,
        camera: bool,
        microphone: bool,
    ) -> Result<TabMediaPermission, CoreError> {
        if !camera && !microphone {
            return Err(CoreError::InvalidInput {
                reason: l10n!(CATALOG, language(), "tabs-media-invalid-resources"),
            });
        }
        let Some((plugin_id, grant)) = self.media_decision(page, origin) else {
            return Ok(TabMediaPermission::Deny);
        };
        match grant {
            Some(true) => return Ok(TabMediaPermission::Allow),
            Some(false) => return Ok(TabMediaPermission::Deny),
            None => {}
        }
        let _writing = lock(&self.writing);
        let mut state = lock(&self.state).clone();
        if state
            .media_prompt
            .as_ref()
            .is_some_and(|pending| pending.place != *page)
        {
            return Ok(TabMediaPermission::Deny);
        }
        state.media_prompt = Some(PendingMediaPrompt {
            place: page.clone(),
            plugin_id,
            camera,
            microphone,
        });
        let plugins = self.plugins_view.borrow().clone();
        let sync_stale = self.sync_problem.borrow().is_some();
        let view = make_view(&state, &plugins, sync_stale);
        *lock(&self.state) = state;
        self.publish(view, &plugins);
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
            return Err(CoreError::InvalidInput {
                reason: l10n!(CATALOG, language(), "tabs-media-no-pending"),
            });
        };
        self.plugins
            .record_media_decision(&pending.plugin_id, allow)?;
        let plugins = self.plugins_view.borrow().clone();
        let sync_stale = self.sync_problem.borrow().is_some();
        let view = make_view(&state, &plugins, sync_stale);
        *lock(&self.state) = state;
        self.publish(view, &plugins);
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
