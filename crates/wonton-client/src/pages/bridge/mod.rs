//! 插件页的 MCP Apps 子集；没有管理接口、聊天发送器或网络客户端。

mod context;
mod rpc;
#[path = "generated/tokens.rs"]
mod tokens;
mod types;

use serde_json::{Value, json};
use wonton_l10n::l10n;
use wonton_protocol::{AppearanceViewModel, CoreError, PanelScope, PlacedTab, TabPage, TabRef};

use super::{PagesLink, placed_tab_ref};
use crate::l10n::{CATALOG, language};
pub(crate) use rpc::handle_message;
pub use types::{TabBridgeAction, TabBridgeResult, TabEnvironment, TabPlatform, TabSafeArea};

const SCRIPT: &str = include_str!("script.js");

pub(crate) fn document_script(
    tabs: &PagesLink,
    appearance: &AppearanceViewModel,
    page: &PlacedTab,
    environment: &TabEnvironment,
) -> Result<String, CoreError> {
    bridge_page(tabs, page)?;
    let payload = context::payload(appearance, environment)?;
    Ok(format!("({SCRIPT})({payload},true);"))
}

pub(crate) fn context_script(
    tabs: &PagesLink,
    appearance: &AppearanceViewModel,
    page: &PlacedTab,
    environment: &TabEnvironment,
) -> Result<String, CoreError> {
    bridge_page(tabs, page)?;
    let payload = context::payload(appearance, environment)?;
    Ok(format!("({SCRIPT})({payload},false);"))
}

pub(crate) fn reselected_script() -> String {
    "window.dispatchEvent(new Event(\"wonton:tab-reselected\"));".to_owned()
}

pub(crate) fn connection_script(connected: bool) -> String {
    let detail = json!({"connected": connected});
    format!("window.dispatchEvent(new CustomEvent(\"wonton:connection\",{{detail:{detail}}}));")
}

fn reply_script(message: Value) -> String {
    format!("window.dispatchEvent(new MessageEvent(\"message\",{{data:{message},source:window}}));")
}

/// 公网网页没有桥、没有注入脚本（ADR-0052“公网网页”）：壳按这个错误不往页面里放。
fn bridge_page(tabs: &PagesLink, page: &PlacedTab) -> Result<TabPage, CoreError> {
    if matches!(placed_tab_ref(page), TabRef::Url { .. }) {
        return Err(public_page());
    }
    page_of(tabs, page)
}

fn public_page() -> CoreError {
    CoreError::InvalidInput {
        reason: l10n!(CATALOG, language(), "tabs-bridge-public-page"),
    }
}

/// 查一个页面实例现在显示什么：栈在 PagesViewModel 里，面板的在面板列表里。
/// 栈、面板和桥都用这一个（ADR-0052：原来查页面的地方有两份，加面板时漏过一次）。
pub(crate) fn page_of(tabs: &PagesLink, placed: &PlacedTab) -> Result<TabPage, CoreError> {
    match placed {
        PlacedTab::Page { tab } => {
            let view = tabs.view();
            view.stack
                .into_iter()
                .find(|item| item.tab == *tab)
                .map(|item| item.page)
                .ok_or_else(page_not_open)
        }
        PlacedTab::Panel { scope, tab } => {
            let view = tabs.view();
            let pages = match scope {
                PanelScope::Main => view.panel,
                PanelScope::Room { room_id } => tabs
                    .room_view()
                    .filter(|view| view.room_id.as_str() == room_id)
                    .map(|view| view.panel.clone())
                    .unwrap_or_default(),
            };
            pages
                .into_iter()
                .find(|item| item.tab == *tab)
                .map(|item| item.page)
                .ok_or_else(page_not_open)
        }
    }
}

fn page_not_open() -> CoreError {
    CoreError::InvalidInput {
        reason: l10n!(CATALOG, language(), "tabs-bridge-page-not-open"),
    }
}
