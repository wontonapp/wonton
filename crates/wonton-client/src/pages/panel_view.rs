//! 面板里的页面：用户放的和 Bot 钉的合在一起。面板页照全屏渲染（ADR-0052：WebView 一直是第 2 级的大小）。

use wonton_protocol::{
    PanelItem, PanelScope, PanelSource, PlacedTab, PluginsViewModel, RoomPanel, TabRef,
};

use super::view::{build_page, build_panel_item};
use super::{PagesState, StoredItem};

/// 主面板：用户自己放的在前，然后是新钉的 Bot 页面。同一个页面只画一次（用用户的）。
pub(super) fn main_panel(state: &PagesState, plugins: &PluginsViewModel) -> Vec<PanelItem> {
    let main = PanelScope::Main;
    let mut items: Vec<PanelItem> = state
        .config
        .panel
        .iter()
        .map(|entry| build_panel_item(state, entry, &main, plugins))
        .collect();
    for pin in state.bot_pins.iter().filter(|pin| pin.scope == main).rev() {
        if !items.iter().any(|item| item.tab == pin.tab) {
            items.push(build_pin_item(state, pin, plugins));
        }
    }
    items
}

/// 房间面板：Bot 钉的在前（新钉的在前），然后用户自己放的；同一个页面只画一次。
pub(super) fn room_panel_items(
    state: &PagesState,
    panel: &RoomPanel,
    scope: &PanelScope,
    plugins: &PluginsViewModel,
) -> Vec<PanelItem> {
    let mut items: Vec<PanelItem> = state
        .bot_pins
        .iter()
        .filter(|pin| pin.scope == *scope)
        .rev()
        .map(|pin| build_pin_item(state, pin, plugins))
        .collect();
    for entry in &panel.pages {
        if !items.iter().any(|item| item.tab == entry.tab) {
            items.push(build_panel_item(state, entry, scope, plugins));
        }
    }
    items
}

/// 一个 Bot 钉的页面：标题用 Bot 给的，不能拖，红点跟着钉页走。
fn build_pin_item(
    state: &PagesState,
    pin: &wonton_protocol::BotPanelPin,
    plugins: &PluginsViewModel,
) -> PanelItem {
    let placed = PlacedTab::Panel {
        scope: pin.scope.clone(),
        tab: pin.tab.clone(),
    };
    let plugin = match &pin.tab {
        TabRef::Plugin { plugin_id, .. }
        | TabRef::PluginPath { plugin_id, .. }
        | TabRef::UiResource { plugin_id, .. } => plugins
            .installed
            .as_ref()
            .and_then(|list| list.plugins.iter().find(|plugin| plugin.id == *plugin_id)),
        _ => None,
    };
    let page_id = match &pin.tab {
        TabRef::Plugin {
            page: Some(page), ..
        } => Some(page.as_str()),
        _ => None,
    };
    let mut page = build_page(
        state,
        &placed,
        &StoredItem {
            tab: pin.tab.clone(),
            page: None,
        },
        plugin,
        page_id,
        plugins,
        Some(pin),
    );
    page.title = pin.title.clone();
    PanelItem {
        tab: pin.tab.clone(),
        page,
        attention: pin.attention,
        source: PanelSource::Bot {
            pin_id: pin.id.clone(),
            plugin_id: pin.plugin_id.clone(),
            bot: pin.bot.clone(),
            name: pin.name.clone(),
        },
        movable: false,
    }
}
