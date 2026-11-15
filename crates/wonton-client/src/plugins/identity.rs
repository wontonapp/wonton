use wonton_matrix::UserId;
use wonton_protocol::{DefaultBotVisibility, InstalledPlugin, InstalledPlugins, PluginDetail};

fn hide_unverified(plugin: &mut InstalledPlugin) {
    let valid_id = plugin
        .bot_user_id
        .as_deref()
        .is_some_and(|id| <&UserId>::try_from(id).is_ok());
    match (plugin.default_bot_visibility, plugin.bot_user_id.is_some()) {
        (Some(DefaultBotVisibility::Visible), true) if valid_id => {}
        (Some(DefaultBotVisibility::Absent | DefaultBotVisibility::Unknown), false) => {}
        _ => {
            if plugin.bot_user_id.is_some()
                || plugin.default_bot_visibility == Some(DefaultBotVisibility::Visible)
            {
                tracing::warn!("default bot identity is unverified or inconsistent");
            }
            plugin.bot_user_id = None;
            plugin.default_bot_visibility = Some(DefaultBotVisibility::Unknown);
        }
    }
}

pub(super) fn invalidate(installed: &mut InstalledPlugins) -> bool {
    let mut changed = false;
    for plugin in &mut installed.plugins {
        changed |= plugin.bot_user_id.take().is_some();
        changed |= plugin.default_bot_visibility != Some(DefaultBotVisibility::Unknown);
        plugin.default_bot_visibility = Some(DefaultBotVisibility::Unknown);
    }
    changed
}

pub(super) fn list(mut installed: InstalledPlugins) -> InstalledPlugins {
    for plugin in &mut installed.plugins {
        hide_unverified(plugin);
    }
    installed
}

pub(super) fn detail(mut detail: PluginDetail) -> PluginDetail {
    if let Some(installed) = &mut detail.installed {
        hide_unverified(installed);
    }
    detail
}
