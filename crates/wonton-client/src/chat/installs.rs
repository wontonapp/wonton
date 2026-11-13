//! 聊天列表最上面的安装占位行（ADR-0067）：有 Bot 的插件在装、在升级、装失败时合成一行，私聊出现后撤掉。
//!
//! 数据都来自插件页的 ViewModel：已安装列表的状态和安装进度、安装任务的结果，加上任务记下的预览（来源和是不是 Bot
//! 插件）。房间列表进到这个模块里判断私聊有没有出现，壳不用去重。

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use tokio::sync::watch;
use wonton_protocol::{
    BotInstallRow, BotInstallState, ChatViewModel, InstallPreview, InstallProgress, InstallStage,
    InstalledPlugins, PluginOperation, PluginState, PluginTask, PluginTaskState, RoomMembership,
    RoomRow,
};

use super::lock;
use crate::plugins::PluginsLink;

/// 装好以后还等不到 Bot 的私聊，过这么久就不占这一行了：插件页照样看得到它。
const SETTLE: Duration = Duration::from_secs(120);

pub(super) struct Installs {
    plugins: Arc<PluginsLink>,
    view: watch::Sender<ChatViewModel>,
    /// 装好、开始等私聊的插件和时刻。
    settling: Mutex<HashMap<String, Instant>>,
    /// 已经有一个计时器在等超时，不再重复起。
    timer: AtomicBool,
}

impl Installs {
    pub(super) fn new(plugins: Arc<PluginsLink>, view: watch::Sender<ChatViewModel>) -> Arc<Self> {
        let installs = Arc::new(Self {
            plugins,
            view,
            settling: Mutex::default(),
            timer: AtomicBool::new(false),
        });
        installs.runtime_spawn_follow();
        installs
    }

    fn runtime_spawn_follow(self: &Arc<Self>) {
        let weak = Arc::downgrade(self);
        let plugins = self.plugins.clone();
        self.plugins.runtime().spawn(async move {
            follow(weak, plugins.subscribe()).await;
        });
    }

    /// 房间列表或插件状态变了：重算这一组行。
    pub(super) fn refresh(self: &Arc<Self>) {
        let (installed, tasks) = {
            let view = self.plugins.view();
            (view.installed, view.tasks)
        };
        let mut settling = lock(&self.settling);
        let now = Instant::now();
        let rows = rows(
            installed.as_ref(),
            &tasks,
            &self.plugins.previews(),
            &self.view.borrow().rooms,
            &mut settling,
            now,
        );
        let waiting = settling
            .values()
            .any(|since| now.duration_since(*since) < SETTLE);
        // 装好以后列表不再刷，要自己到点重算一次，不然那一行会一直挂着。
        if waiting && !self.timer.swap(true, Ordering::SeqCst) {
            let installs = Arc::downgrade(self);
            self.plugins.runtime().spawn(async move {
                tokio::time::sleep(SETTLE).await;
                if let Some(installs) = installs.upgrade() {
                    installs.timer.store(false, Ordering::SeqCst);
                    installs.refresh();
                }
            });
        }
        // 插件表变得频繁，行没变时别惊动壳：时间线订阅跟着发批次，空转时一秒几千批会把主线程占死。
        self.view.send_if_modified(|view| {
            if view.installing_bots == rows {
                return false;
            }
            view.installing_bots = rows;
            true
        });
    }
}

async fn follow(
    installs: Weak<Installs>,
    mut plugins: watch::Receiver<wonton_protocol::PluginsViewModel>,
) {
    loop {
        if let Some(installs) = installs.upgrade() {
            installs.refresh();
        } else {
            return;
        }
        if plugins.changed().await.is_err() {
            return;
        }
    }
}

/// 合成这些行。私聊已经出现（不是退出的）的安装、升级不再留行：那一行相机行接替。
fn rows(
    installed: Option<&InstalledPlugins>,
    tasks: &[PluginTask],
    previews: &HashMap<String, InstallPreview>,
    rooms: &[RoomRow],
    settling: &mut HashMap<String, Instant>,
    now: Instant,
) -> Vec<BotInstallRow> {
    let has_chat = |bot: &str| {
        rooms.iter().any(|room| {
            room.direct_user_id.as_deref() == Some(bot) && room.membership != RoomMembership::Left
        })
    };
    let listed = installed;
    let mut rows = Vec::new();
    let mut covered: HashSet<&str> = HashSet::new();
    if let Some(installed) = installed {
        listed_rows(installed, previews, &has_chat, &mut rows, &mut covered);
    }
    task_rows(
        listed,
        tasks,
        previews,
        &has_chat,
        settling,
        now,
        &mut rows,
        &mut covered,
    );
    rows
}

/// 插件列表里正在装、正在升级的 Bot 插件。
fn listed_rows<'a>(
    installed: &'a InstalledPlugins,
    previews: &HashMap<String, InstallPreview>,
    has_chat: &impl Fn(&str) -> bool,
    rows: &mut Vec<BotInstallRow>,
    covered: &mut HashSet<&'a str>,
) {
    for plugin in &installed.plugins {
        if plugin.chat.is_none() {
            continue;
        }
        let operation = match plugin.status {
            PluginState::Installing => PluginOperation::Install,
            PluginState::Upgrading => PluginOperation::Upgrade,
            _ => continue,
        };
        if operation == PluginOperation::Install
            && plugin.bot_user_id.as_deref().is_some_and(has_chat)
        {
            continue;
        }
        rows.push(BotInstallRow {
            plugin_id: plugin.id.clone(),
            title: plugin.title.clone(),
            icon: plugin.icon.clone(),
            operation,
            source: source(previews, &plugin.id),
            state: BotInstallState::Running {
                progress: plugin.install_progress.clone().unwrap_or(InstallProgress {
                    stage: InstallStage::Other,
                    percent: None,
                }),
            },
        });
        covered.insert(plugin.id.as_str());
    }
}

/// 安装任务自己的结果：列表里还看不到它时的在装、失败和不确定，以及装好后等私聊的那一小段。
#[allow(clippy::too_many_arguments)]
fn task_rows<'a>(
    installed: Option<&InstalledPlugins>,
    tasks: &'a [PluginTask],
    previews: &HashMap<String, InstallPreview>,
    has_chat: &impl Fn(&str) -> bool,
    settling: &mut HashMap<String, Instant>,
    now: Instant,
    rows: &mut Vec<BotInstallRow>,
    covered: &mut HashSet<&'a str>,
) {
    // 重装会换一个新任务，旧的成功时刻不能沿用，否则新的一圈马上被判超时。
    settling.retain(|id, _| {
        tasks
            .iter()
            .any(|task| &task.id == id && task.state == PluginTaskState::Succeeded)
    });
    let bot_of = |id: &str| {
        installed
            .and_then(|installed| installed.plugins.iter().find(|plugin| plugin.id == id))
            .and_then(|plugin| plugin.bot_user_id.as_deref())
    };
    for task in tasks {
        if task.operation == PluginOperation::Uninstall
            || !previews
                .get(&task.id)
                .is_some_and(|preview| preview.has_bot)
        {
            continue;
        }
        if task.operation == PluginOperation::Install && bot_of(&task.id).is_some_and(has_chat) {
            settling.remove(&task.id);
            covered.insert(task.id.as_str());
            continue;
        }
        let state = match &task.state {
            PluginTaskState::Running => {
                // 已安装列表里还看不到它时，也只有这一条能说明在装。
                if covered.contains(task.id.as_str()) {
                    continue;
                }
                BotInstallState::Running {
                    progress: InstallProgress {
                        stage: InstallStage::Other,
                        percent: None,
                    },
                }
            }
            PluginTaskState::Failed { .. } | PluginTaskState::Uncertain { .. } => {
                // 插件已经回滚，可列表多半还是几秒前的“正在装”：按任务的结果来，不留两行。
                rows.retain(|row| row.plugin_id != task.id);
                covered.remove(task.id.as_str());
                match &task.state {
                    PluginTaskState::Failed { reason, message } => BotInstallState::Failed {
                        reason: *reason,
                        message: message.clone(),
                    },
                    _ => match &task.state {
                        PluginTaskState::Uncertain { message, .. } => BotInstallState::Uncertain {
                            message: message.clone(),
                        },
                        _ => unreachable!(),
                    },
                }
            }
            PluginTaskState::Succeeded => {
                if covered.contains(task.id.as_str()) || task.operation == PluginOperation::Upgrade
                {
                    continue;
                }
                let since = *settling.entry(task.id.clone()).or_insert(now);
                if now.duration_since(since) >= SETTLE {
                    settling.remove(&task.id);
                    continue;
                }
                BotInstallState::Running {
                    progress: InstallProgress {
                        stage: InstallStage::WaitingForBot,
                        percent: None,
                    },
                }
            }
        };
        rows.push(BotInstallRow {
            plugin_id: task.id.clone(),
            title: task.title.clone(),
            icon: task.icon.clone(),
            operation: task.operation,
            source: source(previews, &task.id),
            state,
        });
    }
}

fn source(previews: &HashMap<String, InstallPreview>, id: &str) -> Option<String> {
    previews.get(id).and_then(|preview| preview.source.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use wonton_protocol::{
        ChatLayout, InstalledPlugin, PluginChat, PluginFailure, PluginHealth, PluginOperation,
        PluginState, PluginTask, PluginTaskState,
    };

    fn plugin(id: &str, state: PluginState, bot: Option<&str>) -> InstalledPlugin {
        InstalledPlugin {
            chat: Some(PluginChat {
                form: Default::default(),
            }),
            id: id.into(),
            title: format!("标题 {id}"),
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
            web_display: Default::default(),
            cross_origin_isolated: false,
            declares_tools: false,
            tabs: Vec::new(),
            bot_user_id: bot.map(str::to_owned),
            family: false,
            upgrade: None,
            removable: true,
            notices: Vec::new(),
            enabled_root_permissions: Vec::new(),
            install_progress: None,
            tunnel_ports: Vec::new(),
        }
    }

    fn task(id: &str, state: PluginTaskState) -> PluginTask {
        PluginTask {
            id: id.into(),
            title: format!("标题 {id}"),
            icon: None,
            operation: PluginOperation::Install,
            state,
            waiting_for_connection: false,
            added_tools: None,
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
            summary: String::new(),
            license: None,
            license_url: None,
            version: "1.0.0".into(),
            kind: wonton_protocol::PluginKind::Agent,
            runtime: Default::default(),
            security_tier: None,
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
            upgrade_from: None,
            release_notes: None,
            blocked: None,
            blocked_category: None,
            missing_profile: None,
            devices: None,
            lan_webui: None,
            always_allowed: Vec::new(),
        }
    }

    fn room(bot: &str) -> RoomRow {
        RoomRow {
            layout: ChatLayout::default(),
            room_id: format!("!{bot}:home"),
            name: bot.into(),
            is_direct: true,
            direct_user_id: Some(bot.into()),
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
}
