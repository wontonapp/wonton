//! 插件页的 MCP Apps 子集；没有管理接口、聊天发送器或网络客户端。

mod context;
mod rpc;
#[path = "generated/tokens.rs"]
mod tokens;
mod types;

use serde_json::{Value, json};
use wonton_l10n::l10n;
use wonton_protocol::{AppearanceViewModel, CoreError, HomeStatus, TabPage, TabRef};

use super::TabsLink;
use crate::l10n::{CATALOG, language};
pub(crate) use rpc::handle_message;
pub use types::{TabBridgeAction, TabBridgeResult, TabEnvironment, TabPlatform, TabSafeArea};

const SCRIPT: &str = include_str!("script.js");

pub(crate) fn document_script(
    tabs: &TabsLink,
    appearance: &AppearanceViewModel,
    tab: &TabRef,
    environment: &TabEnvironment,
) -> Result<String, CoreError> {
    let payload = context::payload(appearance, &page(tabs, tab)?, environment)?;
    Ok(format!("({SCRIPT})({payload},true);"))
}

pub(crate) fn context_script(
    tabs: &TabsLink,
    appearance: &AppearanceViewModel,
    tab: &TabRef,
    environment: &TabEnvironment,
) -> Result<String, CoreError> {
    let payload = context::payload(appearance, &page(tabs, tab)?, environment)?;
    Ok(format!("({SCRIPT})({payload},false);"))
}

pub(crate) fn reselected_script() -> String {
    "window.dispatchEvent(new Event(\"wonton:tab-reselected\"));".to_owned()
}

pub(crate) fn connection_script(status: &HomeStatus) -> String {
    let detail = json!({"connected": matches!(status, HomeStatus::Connected)});
    format!("window.dispatchEvent(new CustomEvent(\"wonton:connection\",{{detail:{detail}}}));")
}

fn reply_script(message: Value) -> String {
    format!("window.dispatchEvent(new MessageEvent(\"message\",{{data:{message},source:window}}));")
}

fn page(tabs: &TabsLink, tab: &TabRef) -> Result<TabPage, CoreError> {
    let view = tabs.view();
    view.items
        .into_iter()
        .chain(view.overlay)
        .find(|item| item.tab == *tab)
        .and_then(|item| item.page)
        .ok_or_else(|| CoreError::InvalidInput {
            reason: l10n!(CATALOG, language(), "tabs-bridge-page-not-open"),
        })
}
