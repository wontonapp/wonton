use super::*;
use wonton_protocol::{
    InstalledPlugin, InstalledPlugins, PluginFailure, PluginHealth, PluginKind, PluginProblem,
    PluginStart, PluginTrust, RoomMembership,
};

fn plugin(status: PluginState) -> InstalledPlugin {
    InstalledPlugin {
        id: "app.agent".into(),
        title: "Agent".into(),
        summary: String::new(),
        version: "1".into(),
        revision: "1".into(),
        kind: PluginKind::Agent,
        runtime: Default::default(),
        icon: None,
        trust: PluginTrust::Official,
        source: None,
        provider: None,
        via_index: None,
        written_by: None,
        status,
        web_url: None,
        link_url: None,
        cross_origin_isolated: false,
        declares_tools: false,
        tabs: Vec::new(),
        chat: Some(Default::default()),
        bot_user_id: Some("@agent:example.org".into()),
        family: false,
        upgrade: None,
        removable: true,
        notices: Vec::new(),
        enabled_root_permissions: Vec::new(),
        install_progress: None,
        install_failure: None,
        tunnel_ports: Vec::new(),
    }
}

fn rooms() -> Vec<RoomRow> {
    vec![RoomRow {
        room_id: "!dm:example.org".into(),
        name: "Agent".into(),
        is_direct: true,
        direct_user_id: Some("@agent:example.org".into()),
        plugin: None,
        is_encrypted: true,
        is_read_only: false,
        membership: RoomMembership::Joined,
        preview: None,
        side_chats: false,
        unread_count: 0,
        layout: Default::default(),
    }]
}

fn view(status: PluginState) -> PluginsViewModel {
    PluginsViewModel {
        installed: Some(InstalledPlugins {
            plugins: vec![plugin(status)],
            upgrade_check: None,
        }),
        ..Default::default()
    }
}

#[test]
fn stopped_starting_failed_and_running_are_distinct_without_changing_send_permissions() {
    let mut rooms = rooms();
    let mut plugins = view(PluginState::Stopped);
    assert!(refresh(&mut rooms, &plugins));
    let stopped = rooms[0].plugin.as_ref().unwrap();
    assert_eq!(stopped.plugin_id, "app.agent");
    assert_eq!(stopped.status, PluginState::Stopped);
    assert!(stopped.can_start);
    assert!(stopped.start.is_none());
    assert!(!refresh(&mut rooms, &plugins));

    plugins.starts.push(PluginStart {
        plugin_id: "app.agent".into(),
        state: PluginStartState::Starting,
    });
    refresh(&mut rooms, &plugins);
    let starting = rooms[0].plugin.as_ref().unwrap();
    assert_eq!(starting.status, PluginState::Stopped);
    assert_eq!(starting.start, Some(PluginStartState::Starting));
    assert!(!starting.can_start);

    let failed = PluginStartState::Failed {
        problem: PluginProblem {
            reason: PluginFailure::Unauthorized,
            message: "denied".into(),
            min_version: None,
            settings: None,
        },
    };
    plugins.starts[0].state = failed.clone();
    refresh(&mut rooms, &plugins);
    assert_eq!(rooms[0].plugin.as_ref().unwrap().start, Some(failed));
    assert!(rooms[0].plugin.as_ref().unwrap().can_start);
    assert!(!rooms[0].is_read_only);

    plugins.installed.as_mut().unwrap().plugins[0].status = PluginState::Running {
        health: PluginHealth::Healthy,
    };
    refresh(&mut rooms, &plugins);
    assert!(rooms[0].plugin.is_none());
}

#[test]
fn unready_states_remain_visible_but_only_stopped_offers_start() {
    for status in [
        PluginState::Installing,
        PluginState::Upgrading,
        PluginState::Degraded {
            reason: "health".into(),
        },
        PluginState::NeedsConfiguration,
        PluginState::Uninstalling,
        PluginState::Unknown,
    ] {
        let mut rooms = rooms();
        refresh(&mut rooms, &view(status.clone()));
        let shown = rooms[0].plugin.as_ref().unwrap();
        assert_eq!(shown.status, status);
        assert!(!shown.can_start);
    }
}

#[test]
fn unrelated_plugins_groups_and_removed_bots_do_not_get_an_agent_banner() {
    let mut rooms = rooms();
    let mut plugins = view(PluginState::Stopped);
    plugins.starts.push(PluginStart {
        plugin_id: "app.other".into(),
        state: PluginStartState::Starting,
    });
    refresh(&mut rooms, &plugins);
    assert!(rooms[0].plugin.as_ref().unwrap().start.is_none());
    rooms[0].is_direct = false;
    refresh(&mut rooms, &plugins);
    assert!(rooms[0].plugin.is_none());
    rooms[0].is_direct = true;
    rooms[0].direct_user_id = Some("@human:example.org".into());
    refresh(&mut rooms, &plugins);
    assert!(rooms[0].plugin.is_none());
    rooms[0].direct_user_id = Some("@agent:example.org".into());
    refresh(&mut rooms, &plugins);
    plugins.installed = None;
    refresh(&mut rooms, &plugins);
    assert!(rooms[0].plugin.is_none());
}
