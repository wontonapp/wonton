//! 插件私聊的可用性只由插件列表和本设备的启动请求决定，不改变 Matrix 发送权限。

use wonton_protocol::{
    InstalledPlugin, PluginStartState, PluginState, PluginsViewModel, RoomPlugin, RoomRow,
};

pub(super) fn refresh(rooms: &mut [RoomRow], plugins: &PluginsViewModel) -> bool {
    let mut changed = false;
    for room in rooms {
        let Some((plugin, start)) = for_room(room, plugins) else {
            changed |= room.plugin.take().is_some();
            continue;
        };
        let can_start = matches!(plugin.status, PluginState::Stopped)
            && !matches!(start, Some(PluginStartState::Starting));
        if room.plugin.as_ref().is_some_and(|current| {
            current.plugin_id == plugin.id
                && current.status == plugin.status
                && current.start.as_ref() == start
                && current.can_start == can_start
        }) {
            continue;
        }
        room.plugin = Some(RoomPlugin {
            plugin_id: plugin.id.clone(),
            status: plugin.status.clone(),
            start: start.cloned(),
            can_start,
        });
        changed = true;
    }
    changed
}

fn for_room<'a>(
    room: &RoomRow,
    plugins: &'a PluginsViewModel,
) -> Option<(&'a InstalledPlugin, Option<&'a PluginStartState>)> {
    if !room.is_direct {
        return None;
    }
    let user = room.direct_user_id.as_deref()?;
    let plugin = plugins
        .installed
        .as_ref()?
        .plugins
        .iter()
        .find(|plugin| plugin.bot_user_id.as_deref() == Some(user))?;
    if matches!(plugin.status, PluginState::Running { .. }) {
        return None;
    }
    let start = plugins
        .starts
        .iter()
        .find(|start| start.plugin_id == plugin.id)
        .map(|start| &start.state);
    Some((plugin, start))
}

#[cfg(test)]
#[path = "plugin_status_tests.rs"]
mod plugin_status_tests;
