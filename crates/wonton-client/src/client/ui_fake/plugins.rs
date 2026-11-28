//! 假设备页和应用网格：商店入口、没有网页也没有 Bot 的应用、有网页的几个应用，以及“我的应用”里的两行。

use wonton_protocol::{
    AppsAction, InstalledPlugin, InstalledPlugins, PluginHealth, PluginKind, PluginState,
    PluginTrust, PluginsViewModel, SidebarApp, SidebarBadge, SidebarOpen, StoreLoadState,
};

use super::tabs::{CALL_PLUGIN, FakePage, WEB_APPS, web_app_tab, web_url};

/// 应用网格上有东西可点：商店入口，一个没有网页也没有 Bot 的应用（点图标进它的详情页），
/// 和 `WEB_APPS` 里有网页的几个（点图标整页打开）。
pub(in crate::client) fn fake_plugins(mut plugins: PluginsViewModel) -> PluginsViewModel {
    plugins.actions = vec![AppsAction::Store, AppsAction::AddSource];
    let mut installed = vec![fake_plugin(
        "Notes",
        "app.example.notes",
        PluginState::Stopped,
        None,
    )];
    installed.extend(WEB_APPS.iter().map(|(title, id, kind)| {
        let status = match kind {
            FakePage::Stopped => PluginState::Stopped,
            FakePage::Loading | FakePage::Failed | FakePage::Local => PluginState::Running {
                health: PluginHealth::Healthy,
            },
        };
        fake_plugin(title, id, status, Some(web_url(id)))
    }));
    installed.push(fake_plugin(
        "Autobangumi",
        &super::super::ui_fake_install::consumer_id(),
        PluginState::Degraded {
            reason: "qBittorrent isn't running".into(),
        },
        None,
    ));
    installed.push(fake_plugin(
        "qBittorrent",
        &super::super::ui_fake_install::provider_id(),
        PluginState::Stopped,
        None,
    ));
    installed.push(fake_plugin(
        super::super::ui_fake_plugin_detail::HUB_TITLE,
        super::super::ui_fake_plugin_detail::HUB_ID,
        super::super::ui_fake_plugin_detail::status(),
        Some(web_url(super::super::ui_fake_plugin_detail::HUB_ID)),
    ));
    installed.extend(super::super::ui_fake_failed_install::plugin());
    installed.push(super::music::plugin());
    plugins.installed = Some(InstalledPlugins {
        plugins: installed,
        upgrade_check: None,
    });
    plugins.store = Some(super::super::ui_fake_store::listing());
    plugins.store_load = StoreLoadState::Ready;
    plugins.problem = None;
    // 我的应用里放两个应用：一个有分享、一个没有，设置里的行点开就是详情页。
    plugins.sidebar.apps = vec![
        sidebar_app(
            super::super::ui_fake_plugin_detail::BUDGET_TITLE,
            Some("Household Budget"),
            super::super::ui_fake_plugin_detail::BUDGET_ID,
        ),
        sidebar_app(
            super::super::ui_fake_plugin_detail::HUB_TITLE,
            None,
            super::super::ui_fake_plugin_detail::HUB_ID,
        ),
    ];
    // 通话插件的端口映射只连上了公共中继：它的 widget 页上有中继提示可看（ADR-0096 第 4 节）。
    plugins.port_mappings = vec![wonton_protocol::PluginPortMapping {
        plugin_id: CALL_PLUGIN.into(),
        enabled: false,
        state: wonton_protocol::PortMappingState::On,
        ports: Vec::new(),
        warning: Some(wonton_protocol::PortMappingWarning::PublicRelayBandwidth),
    }];
    plugins
}

fn sidebar_app(title: &str, subtitle: Option<&str>, id: &str) -> SidebarApp {
    SidebarApp {
        plugin_id: Some(id.into()),
        tab: None,
        title: title.into(),
        subtitle: subtitle.map(Into::into),
        open: SidebarOpen::Page {
            tab: web_app_tab(id),
        },
        icon: None,
        badge: SidebarBadge::default(),
    }
}

pub(in crate::client) fn fake_plugin(
    title: &str,
    id: &str,
    status: PluginState,
    web_url: Option<String>,
) -> InstalledPlugin {
    InstalledPlugin {
        bot_shared: false,
        bot_danger: false,
        bot_danger_can_toggle: false,
        id: id.into(),
        title: title.into(),
        subtitle: None,
        summary: String::new(),
        version: "1.0.0".into(),
        revision: "1".into(),
        kind: PluginKind::App,
        runtime: Default::default(),
        icon: None,
        trust: PluginTrust::Official,
        source: None,
        provider: None,
        via_index: None,
        written_by: None,
        status,
        web_url,
        link_url: None,
        cross_origin_isolated: false,
        declares_tools: false,
        tabs: Vec::new(),
        chat: None,
        bot_user_id: None,
        default_bot_visibility: None,
        upgrade: None,
        removable: true,
        notices: Vec::new(),
        enabled_root_permissions: Vec::new(),
        install_progress: None,
        install_failure: None,
        tunnel_ports: Vec::new(),
        media_browse: None,
        matrix_widget: None,
    }
}
