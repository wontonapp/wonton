//! 页面测试共用的假插件、页面和断言。

use wonton_client::{
    CoreError, InstalledPlugin, PagesViewModel, PluginImage, PluginState, PluginTab, TabLayout,
    TabPage, TabRef,
};

use crate::plugins::rig::installed;

pub(super) fn page(id: &str) -> PluginTab {
    PluginTab {
        id: id.to_owned(),
        title: format!("页面 {id}"),
        icon: PluginImage {
            path: format!("/_wonton/api/images/{id}.png"),
        },
        icon_selected: None,
        path: format!("/{id}"),
        layout: TabLayout::Inset,
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

/// 栈里那页的渲染数据；不在栈里就失败。
pub(super) fn item<'a>(view: &'a PagesViewModel, tab: &TabRef) -> &'a TabPage {
    view.stack
        .iter()
        .find(|open| open.tab == *tab)
        .map(|open| &open.page)
        .unwrap_or_else(|| panic!("栈里没有 {tab:?}: {view:?}"))
}

pub(super) fn assert_invalid(result: Result<(), CoreError>) {
    assert!(
        matches!(result, Err(CoreError::InvalidInput { .. })),
        "{result:?}"
    );
}
