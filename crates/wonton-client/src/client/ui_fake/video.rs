//! 假数据入口里的影视库：一个声明了 `catalog` 的媒体源，点开是原生影视页（片子见 `ui_fake_video`），
//! 胶囊菜单里的“网页版”和别的假应用一样停在骨架（没有隧道）。

use wonton_protocol::media::MediaOperation;
use wonton_protocol::{
    InstalledPlugin, NativePage, PluginHealth, PluginState, TabIcon, TabLayout, TabPage, TabRef,
    TabState,
};

use super::plugins::fake_plugin;
use super::tabs::web_url;

const VIDEO_PLUGIN: &str = "org.jellyfin.media";
const VIDEO_TITLE: &str = "Jellyfin";

/// 应用网格里的影视库。
pub(super) fn plugin() -> InstalledPlugin {
    let mut plugin = fake_plugin(
        VIDEO_TITLE,
        VIDEO_PLUGIN,
        PluginState::Running {
            health: PluginHealth::Healthy,
        },
        Some(web_url(VIDEO_PLUGIN)),
    );
    plugin.media_operations = vec![
        MediaOperation::List,
        MediaOperation::Playback,
        MediaOperation::Download,
        MediaOperation::Catalog,
    ];
    plugin
}

/// 打开影视库的入口是原生界面，“网页版”是网页。
pub(super) fn page(tab: &TabRef) -> Option<TabPage> {
    let native = match tab {
        TabRef::Plugin { plugin_id, page } if plugin_id == VIDEO_PLUGIN => {
            page.is_none().then_some(NativePage::Video)
        }
        TabRef::PluginPath { plugin_id, .. } if plugin_id == VIDEO_PLUGIN => None,
        _ => return None,
    };
    Some(TabPage {
        title: VIDEO_TITLE.into(),
        icon: TabIcon::Plugin { icon: None },
        url: Some(web_url(VIDEO_PLUGIN)),
        layout: TabLayout::default(),
        state: TabState::Ready,
        reload: 0,
        opens_in_browser: false,
        native,
    })
}
