//! 底栏测试共用的假插件、页面和断言。

use wonton_client::{
    CoreError, InstalledPlugin, PluginImage, PluginState, PluginTab, TabDisplay, TabLayout, TabRef,
    TabsViewModel,
};

use crate::plugins::rig::installed;

pub(super) fn page(id: &str, display: TabDisplay) -> PluginTab {
    PluginTab {
        id: id.to_owned(),
        title: format!("页面 {id}"),
        icon: PluginImage {
            path: format!("/_wonton/api/images/{id}.png"),
        },
        icon_selected: None,
        path: format!("/{id}"),
        layout: TabLayout::Inset,
        display,
    }
}

pub(super) fn web_plugin(id: &str, status: PluginState, tabs: Vec<PluginTab>) -> InstalledPlugin {
    let mut plugin = installed(id, status);
    plugin.web_url = Some(format!("http://{id}.wonton.localhost/"));
    plugin.tabs = tabs;
    plugin
}

pub(super) fn tab(plugin_id: &str, page: Option<&str>) -> TabRef {
    TabRef::Plugin {
        plugin_id: plugin_id.to_owned(),
        page: page.map(str::to_owned),
    }
}

pub(super) fn item<'a>(view: &'a TabsViewModel, tab: &TabRef) -> &'a wonton_client::TabItem {
    view.items
        .iter()
        .find(|item| item.tab == *tab)
        .unwrap_or_else(|| panic!("底栏里没有 {tab:?}: {view:?}"))
}

pub(super) fn assert_invalid(result: Result<(), CoreError>) {
    assert!(
        matches!(result, Err(CoreError::InvalidInput { .. })),
        "{result:?}"
    );
}
