use super::*;
use wonton_protocol::{
    MyAppRef, PluginFailure, PluginHealth, PluginStartState, PluginState, RoomPlugin,
};

const ID: &str = "app.agent";

async fn room_plugin(
    phone: &Phone,
    room: &str,
    matches: impl Fn(&Option<RoomPlugin>) -> bool,
) -> Option<RoomPlugin> {
    phone
        .wait_chat("plugin lifecycle in direct room", |view| {
            let row = view.rooms.iter().find(|row| row.room_id == room)?;
            matches(&row.plugin).then(|| row.plugin.clone())
        })
        .await
}

async fn wait_until(check: impl Fn() -> bool) {
    tokio::time::timeout(WAIT, async {
        while !check() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("lifecycle side effect");
}

async fn stopped_setup() -> (
    tempfile::TempDir,
    Phone,
    Home,
    OwnedRoomId,
    wonton_client::InstalledPlugin,
) {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    phone.signed_in().await;
    let bot = home.bot("agent", "Agent").await;
    let room = accept_the_bots_invite(&home, &phone, &bot).await;
    let mut plugin = crate::plugins::rig::installed(ID, PluginState::Stopped);
    plugin.kind = wonton_client::PluginKind::Agent;
    plugin.bot_user_id = Some(bot.user_id().to_string());
    plugin.chat = Some(Default::default());
    home.set_installed_plugins(vec![plugin.clone()]);
    phone.dispatch(Action::RefreshPlugins).unwrap();
    let stopped = room_plugin(&phone, room.as_str(), |plugin| {
        plugin
            .as_ref()
            .is_some_and(|plugin| plugin.status == PluginState::Stopped)
    })
    .await
    .unwrap();
    assert_eq!(stopped.plugin_id, ID);
    assert!(stopped.can_start);
    phone
        .dispatch(Action::AddMyApp {
            app: MyAppRef::Plugin {
                plugin_id: ID.into(),
            },
            title: None,
            index: None,
        })
        .unwrap();
    wait_until(|| {
        phone
            .plugins()
            .sidebar
            .apps
            .iter()
            .any(|app| app.plugin_id.as_deref() == Some(ID) && app.badge.stopped)
    })
    .await;
    (dir, phone, home, room, plugin)
}

async fn start_and_observe_failure(home: &Home, phone: &Phone, room: &str) {
    home.hold_plugin_starts(true);
    home.answer_plugin_start(
        500,
        serde_json::json!({"error": {"kind": "upstream", "message": "unhealthy"}}),
    );
    phone
        .dispatch(Action::StartPlugin {
            plugin_id: ID.into(),
        })
        .unwrap();
    let starting = room_plugin(phone, room, |plugin| {
        plugin
            .as_ref()
            .is_some_and(|plugin| plugin.start == Some(PluginStartState::Starting))
    })
    .await
    .unwrap();
    assert!(!starting.can_start);
    assert_eq!(starting.status, PluginState::Stopped);
    wait_until(|| home.plugin_starts().len() == 1).await;
    assert_eq!(home.plugin_starts()[0], (ID.into(), serde_json::json!({})));
    assert!(matches!(
        phone.dispatch(Action::StartPlugin {
            plugin_id: ID.into()
        }),
        Err(wonton_client::CoreError::NotReady { .. })
    ));
    home.hold_plugin_starts(false);
    let failed = room_plugin(phone, room, |plugin| {
        plugin.as_ref().is_some_and(|plugin| matches!(&plugin.start, Some(PluginStartState::Failed { problem }) if problem.reason == PluginFailure::Upstream))
    }).await.unwrap();
    assert!(failed.can_start);
    let before = home.installed_requests();
    phone.dispatch(Action::RefreshPlugins).unwrap();
    wait_until(|| home.installed_requests() > before).await;
    let refreshed = room_plugin(phone, room, |plugin| plugin.as_ref() == Some(&failed)).await;
    assert_eq!(refreshed, Some(failed));
}

async fn recover_externally(
    home: &Home,
    phone: &Phone,
    room: &str,
    stopped: &wonton_client::InstalledPlugin,
) {
    let mut running = stopped.clone();
    running.status = PluginState::Running {
        health: PluginHealth::Healthy,
    };
    home.set_installed_plugins(vec![running]);
    phone.dispatch(Action::RefreshPlugins).unwrap();
    room_plugin(phone, room, Option::is_none).await;
    assert!(phone.plugins().starts.is_empty());

    home.set_installed_plugins(vec![stopped.clone()]);
    phone.dispatch(Action::RefreshPlugins).unwrap();
    let reset = room_plugin(phone, room, |plugin| {
        plugin
            .as_ref()
            .is_some_and(|plugin| plugin.status == PluginState::Stopped && plugin.start.is_none())
    })
    .await
    .unwrap();
    assert!(reset.can_start);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，Agent 私聊经真实 Matrix 同步后启动插件"]
async fn stopped_agent_can_start_retry_and_return_to_running_in_the_same_room() {
    let (_dir, phone, home, room, mut plugin) = stopped_setup().await;

    start_and_observe_failure(&home, &phone, room.as_str()).await;
    recover_externally(&home, &phone, room.as_str(), &plugin).await;

    plugin.status = PluginState::Running {
        health: PluginHealth::Healthy,
    };
    home.answer_plugin_start(200, serde_json::to_value(&plugin).unwrap());
    home.set_installed_plugins(vec![plugin]);
    phone
        .dispatch(Action::StartPlugin {
            plugin_id: ID.into(),
        })
        .unwrap();
    wait_until(|| home.plugin_starts().len() == 2 && phone.plugins().starts.is_empty()).await;
    room_plugin(&phone, room.as_str(), Option::is_none).await;
    assert_eq!(home.plugin_starts().len(), 2);
    wait_until(|| {
        phone
            .plugins()
            .sidebar
            .apps
            .iter()
            .any(|app| app.plugin_id.as_deref() == Some(ID) && !app.badge.stopped)
    })
    .await;
    assert!(phone.plugins().starts.is_empty());
    assert!(
        !phone
            .chat()
            .rooms
            .iter()
            .find(|row| row.room_id == room.as_str())
            .unwrap()
            .is_read_only
    );
}
