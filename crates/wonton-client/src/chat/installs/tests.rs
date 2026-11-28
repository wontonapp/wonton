use std::collections::HashMap;
use std::time::{Duration, Instant};

use wonton_protocol::{
    BotInstallRow, BotInstallState, ChatLayout, InstallPreview, InstallProgress, InstallStage,
    InstalledPlugin, InstalledPlugins, PluginChat, PluginFailure, PluginHealth, PluginOperation,
    PluginState, PluginTask, PluginTaskState, RoomMembership, RoomRow,
};

use super::{SETTLE, rows::rows};
use crate::plugins::PluginsLink;

fn plugin(id: &str, state: PluginState, bot: Option<&str>) -> InstalledPlugin {
    InstalledPlugin {
        bot_danger_can_toggle: false,
        bot_shared: false,
        bot_danger: false,
        chat: Some(PluginChat {
            form: Default::default(),
        }),
        id: id.into(),
        title: format!("标题 {id}"),
        subtitle: None,
        summary: String::new(),
        version: "1.0.0".into(),
        revision: "1.0.0".into(),
        kind: wonton_protocol::PluginKind::Agent,
        runtime: Default::default(),
        icon: None,
        trust: wonton_protocol::PluginTrust::Official,
        source: None,
        provider: None,
        via_index: None,
        written_by: None,
        status: state,
        web_url: None,
        link_url: None,
        cross_origin_isolated: false,
        declares_tools: false,
        tabs: Vec::new(),
        bot_user_id: bot.map(str::to_owned),
        default_bot_visibility: bot.map(|_| wonton_protocol::DefaultBotVisibility::Visible),
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

fn task(id: &str, state: PluginTaskState) -> PluginTask {
    PluginTask {
        id: id.into(),
        title: format!("标题 {id}"),
        icon: None,
        operation: PluginOperation::Install,
        state,
        install_progress: None,
        waiting_for_connection: false,
        added_tools: None,
        dependents: Vec::new(),
    }
}

fn preview(id: &str, has_bot: bool) -> InstallPreview {
    InstallPreview {
        source: Some(format!("github:owner/{id}@v1.0.0")),
        commit: None,
        digest: "sha256:abc".into(),
        plan: "opaque-install-plan".into(),
        id: id.into(),
        title: format!("标题 {id}"),
        subtitle: None,
        summary: String::new(),
        license: None,
        license_url: None,
        version: "1.0.0".into(),
        kind: wonton_protocol::PluginKind::Agent,
        runtime: Default::default(),
        security_tier: None,
        remote: None,
        scope_choices: Vec::new(),
        has_bot,
        icon: None,
        trust: wonton_protocol::PluginTrust::Official,
        provider: None,
        via_index: None,
        written_by: None,
        author: None,
        permissions: Vec::new(),
        widget_capabilities: Vec::new(),
        secret_inputs: Vec::new(),
        settings: None,
        notices: Vec::new(),
        missing_requires: Vec::new(),
        missing_wants: Vec::new(),
        wanting_plugins: Vec::new(),
        connected_plugins: Vec::new(),
        upgrade_from: None,
        release_notes: None,
        blocked: None,
        blocked_category: None,
        missing_profile: None,
        devices: None,
        lan_webui: None,
        always_allowed: Vec::new(),
        audience: None,
        install_gate: wonton_protocol::Gate::Allow,
        install_providers: Vec::new(),
    }
}

fn room(bot: &str) -> RoomRow {
    RoomRow {
        unread_mentions: 0,
        layout: ChatLayout::default(),
        call_active: false,
        room_id: format!("!{bot}:home"),
        name: bot.into(),
        is_direct: true,
        direct_user_id: Some(bot.into()),
        plugin: None,
        bot_chat: None,
        bot: None,
        is_encrypted: true,
        is_read_only: false,
        membership: RoomMembership::Joined,
        preview: None,
        side_chats: false,
        unread_count: 0,
    }
}

fn installed(plugins: Vec<InstalledPlugin>) -> InstalledPlugins {
    InstalledPlugins {
        plugins,
        upgrade_check: None,
    }
}

fn progress(stage: InstallStage) -> Option<InstallProgress> {
    Some(InstallProgress {
        stage,
        percent: Some(40),
    })
}

fn run(
    plugins: Vec<InstalledPlugin>,
    tasks: &[PluginTask],
    previews: &[InstallPreview],
    rooms: &[RoomRow],
    settling: &mut HashMap<String, Instant>,
    now: Instant,
) -> Vec<BotInstallRow> {
    let previews = previews
        .iter()
        .map(|preview| (preview.id.clone(), preview.clone()))
        .collect();
    rows(
        Some(&installed(plugins)),
        tasks,
        &previews,
        rooms,
        settling,
        now,
    )
}

fn failed_record() -> InstalledPlugin {
    let mut value = serde_json::to_value(plugin(
        "app.hermes",
        PluginState::Degraded {
            reason: "pull failed; cleanup failed".into(),
        },
        None,
    ))
    .unwrap();
    value["source"] = serde_json::json!("github:owner/hermes@v1.0.0");
    value["install_failure"] = serde_json::json!({
        "kind": "image_pull_failed",
        "message": "pull failed",
        "stage": "pulling_images",
        "failed_plugin_id": "app.hermes",
        "cleanup_errors": ["cleanup failed"]
    });
    serde_json::from_value(value).unwrap()
}

#[test]
fn server_failure_restores_a_retryable_row_without_local_task_or_preview() {
    let failed = failed_record();
    let expected_source = failed.source.clone();
    let rows = run(
        vec![failed],
        &[],
        &[],
        &[],
        &mut HashMap::new(),
        Instant::now(),
    );
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].source, expected_source);
    assert_eq!(rows[0].operation, PluginOperation::Install);
    assert_eq!(
        rows[0].state,
        BotInstallState::Failed {
            reason: PluginFailure::SourceUnavailable,
            message: "pull failed; cleanup failed".into(),
        }
    );
}

#[test]
fn server_failure_overrides_stale_uncertain_and_successful_tasks() {
    for state in [
        PluginTaskState::Uncertain {
            message: "connection lost".into(),
            next_step: wonton_protocol::PluginTaskNextStep::CheckInstalled,
        },
        PluginTaskState::Succeeded,
    ] {
        let mut settling = HashMap::from([("app.hermes".into(), Instant::now())]);
        let rows = run(
            vec![failed_record()],
            &[task("app.hermes", state)],
            &[preview("app.hermes", true)],
            &[],
            &mut settling,
            Instant::now(),
        );
        assert_eq!(rows.len(), 1);
        assert!(matches!(
            rows[0].state,
            BotInstallState::Failed {
                reason: PluginFailure::SourceUnavailable,
                ..
            }
        ));
        assert_eq!(
            rows[0].source.as_deref(),
            Some("github:owner/hermes@v1.0.0")
        );
        assert!(settling.is_empty());
    }
}

#[test]
fn successful_retry_is_not_reclassified_by_the_previous_failure_snapshot() {
    let mut view = wonton_protocol::PluginsViewModel {
        installed: Some(installed(vec![failed_record()])),
        tasks: vec![task("app.hermes", PluginTaskState::Succeeded)],
        ..Default::default()
    };
    PluginsLink::derive(&mut view);
    assert_eq!(view.tasks[0].state, PluginTaskState::Succeeded);
    assert!(view.updatable.is_empty());
    assert!(!PluginsLink::derive(&mut view));
}

#[test]
fn active_retry_supersedes_the_cached_failure_row() {
    let rows = run(
        vec![failed_record()],
        &[task("app.hermes", PluginTaskState::Running)],
        &[preview("app.hermes", true)],
        &[],
        &mut HashMap::new(),
        Instant::now(),
    );
    assert_eq!(rows.len(), 1);
    assert!(matches!(rows[0].state, BotInstallState::Running { .. }));
    assert_eq!(rows[0].source, preview("app.hermes", true).source);
}

#[test]
fn an_installing_bot_plugin_gets_its_stage_row() {
    let mut installing = plugin("app.hermes", PluginState::Installing, None);
    installing.install_progress = progress(InstallStage::PullingImages);
    let rows = run(
        vec![installing],
        &[],
        &[preview("app.hermes", true)],
        &[],
        &mut HashMap::new(),
        Instant::now(),
    );
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].title, "标题 app.hermes");
    assert_eq!(rows[0].operation, PluginOperation::Install);
    assert_eq!(
        rows[0].source.as_deref(),
        Some("github:owner/app.hermes@v1.0.0")
    );
    assert!(matches!(
        &rows[0].state,
        BotInstallState::Running { progress }
            if progress.stage == InstallStage::PullingImages && progress.percent == Some(40)
    ));
}

#[test]
fn plugins_without_a_bot_and_other_states_stay_out() {
    let mut plain = plugin("app.photos", PluginState::Installing, None);
    plain.chat = None;
    let rows = run(
        vec![
            plain,
            plugin(
                "app.memos",
                PluginState::Running {
                    health: PluginHealth::Healthy,
                },
                None,
            ),
        ],
        &[],
        &[preview("app.memos", true)],
        &[],
        &mut HashMap::new(),
        Instant::now(),
    );
    assert!(rows.is_empty(), "{rows:?}");
}

#[test]
fn a_failed_install_shows_its_category_and_retry_source() {
    let rows = run(
        Vec::new(),
        &[task(
            "app.hermes",
            PluginTaskState::Failed {
                reason: PluginFailure::TransportUnavailable,
                message: "连不上".into(),
            },
        )],
        &[preview("app.hermes", true)],
        &[],
        &mut HashMap::new(),
        Instant::now(),
    );
    assert_eq!(
        rows[0].state,
        BotInstallState::Failed {
            reason: PluginFailure::TransportUnavailable,
            message: "连不上".into(),
        }
    );
    assert_eq!(
        rows[0].source.as_deref(),
        Some("github:owner/app.hermes@v1.0.0")
    );

    let rows = run(
        Vec::new(),
        &[task(
            "app.hermes",
            PluginTaskState::Uncertain {
                message: "不知道".into(),
                next_step: wonton_protocol::PluginTaskNextStep::CheckInstalled,
            },
        )],
        &[preview("app.hermes", true)],
        &[],
        &mut HashMap::new(),
        Instant::now(),
    );
    assert_eq!(
        rows[0].state,
        BotInstallState::Uncertain {
            message: "不知道".into(),
        }
    );
}

#[test]
fn a_plain_agent_install_writes_no_chat_row() {
    let rows = run(
        Vec::new(),
        &[task("app.tool", PluginTaskState::Succeeded)],
        &[preview("app.tool", false)],
        &[],
        &mut HashMap::new(),
        Instant::now(),
    );
    assert!(rows.is_empty());
}

#[test]
fn the_bot_chat_replaces_the_install_row() {
    let rows = run(
        vec![plugin(
            "app.hermes",
            PluginState::Running {
                health: PluginHealth::Healthy,
            },
            Some("@hermes:home"),
        )],
        &[task("app.hermes", PluginTaskState::Succeeded)],
        &[preview("app.hermes", true)],
        &[room("@hermes:home")],
        &mut HashMap::new(),
        Instant::now(),
    );
    assert!(rows.is_empty(), "私聊出现了就不留安装行");

    let mut installing = plugin("app.hermes", PluginState::Installing, Some("@hermes:home"));
    installing.install_progress = progress(InstallStage::Starting);
    let rows = run(
        vec![installing],
        &[],
        &[preview("app.hermes", true)],
        &[room("@hermes:home")],
        &mut HashMap::new(),
        Instant::now(),
    );
    assert!(rows.is_empty(), "Bot 已经建好、私聊已在，安装行让位");
}

#[test]
fn after_success_it_waits_for_the_chat_then_gives_up() {
    let now = Instant::now();
    let mut settling = HashMap::new();
    let tasks = [task("app.hermes", PluginTaskState::Succeeded)];
    let previews = [preview("app.hermes", true)];
    let rows = run(Vec::new(), &tasks, &previews, &[], &mut settling, now);
    assert!(matches!(
        &rows[0].state,
        BotInstallState::Running { progress } if progress.stage == InstallStage::WaitingForBot
    ));

    let later = now + SETTLE + Duration::from_secs(1);
    let rows = run(Vec::new(), &tasks, &previews, &[], &mut settling, later);
    assert!(rows.is_empty(), "等太久了就不再占这一行");
}

#[test]
fn an_upgrade_keeps_its_row_while_the_chat_exists() {
    let mut upgrading = plugin("app.hermes", PluginState::Upgrading, Some("@hermes:home"));
    upgrading.install_progress = progress(InstallStage::Starting);
    let rows = run(
        vec![upgrading],
        &[],
        &[preview("app.hermes", true)],
        &[room("@hermes:home")],
        &mut HashMap::new(),
        Instant::now(),
    );
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].operation, PluginOperation::Upgrade);
}

#[test]
fn one_row_per_plugin_even_with_a_running_task() {
    let mut installing = plugin("app.hermes", PluginState::Installing, None);
    installing.install_progress = progress(InstallStage::PullingImages);
    let rows = run(
        vec![installing],
        &[task("app.hermes", PluginTaskState::Running)],
        &[preview("app.hermes", true)],
        &[],
        &mut HashMap::new(),
        Instant::now(),
    );
    assert_eq!(rows.len(), 1);
    assert!(matches!(
        &rows[0].state,
        BotInstallState::Running { progress } if progress.stage == InstallStage::PullingImages
    ));
}

#[test]
fn a_failure_replaces_the_stale_installing_row() {
    let mut installing = plugin("app.hermes", PluginState::Installing, None);
    installing.install_progress = progress(InstallStage::Starting);
    let rows = run(
        vec![installing],
        &[task(
            "app.hermes",
            PluginTaskState::Failed {
                reason: PluginFailure::Upstream,
                message: "Docker 报错".into(),
            },
        )],
        &[preview("app.hermes", true)],
        &[],
        &mut HashMap::new(),
        Instant::now(),
    );
    assert_eq!(rows.len(), 1, "几秒前的“正在装”不能和失败并排");
    assert!(matches!(rows[0].state, BotInstallState::Failed { .. }));
}

#[test]
fn a_new_attempt_restarts_the_wait_for_the_chat() {
    let now = Instant::now();
    let mut settling = HashMap::new();
    settling.insert(
        "app.hermes".to_owned(),
        now - SETTLE - Duration::from_secs(5),
    );
    let rows = run(
        Vec::new(),
        &[task("app.hermes", PluginTaskState::Running)],
        &[preview("app.hermes", true)],
        &[],
        &mut settling,
        now,
    );
    assert_eq!(rows.len(), 1, "新一轮在装");
    assert!(!settling.contains_key("app.hermes"), "旧的成功时刻作废");
}

#[test]
fn a_running_task_without_the_plugin_listed_still_shows() {
    let rows = run(
        Vec::new(),
        &[task("app.hermes", PluginTaskState::Running)],
        &[preview("app.hermes", true)],
        &[],
        &mut HashMap::new(),
        Instant::now(),
    );
    assert_eq!(rows.len(), 1);
    assert!(matches!(
        &rows[0].state,
        BotInstallState::Running { progress } if progress.stage == InstallStage::Other
    ));
}
