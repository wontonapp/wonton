//! 假数据入口里的相册应用：点开是原生相册页（`NativePage::Photos`），内容是 `ui_fake_photos` 的假相册；
//! “网页版”和别的假应用一样停在骨架（没有隧道）。

use wonton_protocol::{
    InstalledPlugin, NativePage, PluginHealth, PluginState, TabIcon, TabLayout, TabPage, TabRef,
    TabState,
};

use super::plugins::fake_plugin;
use super::tabs::web_url;

const PHOTOS_PLUGIN: &str = "app.immich.immich";
const PHOTOS_TITLE: &str = "Immich";

/// 应用网格里的相册应用。
pub(super) fn plugin() -> InstalledPlugin {
    fake_plugin(
        PHOTOS_TITLE,
        PHOTOS_PLUGIN,
        PluginState::Running {
            health: PluginHealth::Healthy,
        },
        Some(web_url(PHOTOS_PLUGIN)),
    )
}

/// 打开相册应用的入口是原生界面，“网页版”是网页。
pub(super) fn page(tab: &TabRef) -> Option<TabPage> {
    let native = match tab {
        TabRef::Plugin { plugin_id, page } if plugin_id == PHOTOS_PLUGIN => {
            page.is_none().then_some(NativePage::Photos)
        }
        TabRef::PluginPath { plugin_id, .. } if plugin_id == PHOTOS_PLUGIN => None,
        _ => return None,
    };
    Some(TabPage {
        title: PHOTOS_TITLE.into(),
        icon: TabIcon::Plugin { icon: None },
        url: Some(web_url(PHOTOS_PLUGIN)),
        layout: TabLayout::default(),
        state: TabState::Ready,
        reload: 0,
        opens_in_browser: false,
        native,
    })
}
