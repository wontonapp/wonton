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
        let plugins = self.plugins.view();
        let mut settling = lock(&self.settling);
        let now = Instant::now();
        let rows = rows(
            plugins.installed.as_ref(),
            &plugins.tasks,
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
            let changed = super::plugin_status::refresh(&mut view.rooms, &plugins);
            if view.installing_bots == rows {
                return changed;
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
        listed_rows(
            installed,
            tasks,
            previews,
            &has_chat,
            &mut rows,
            &mut covered,
        );
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
    tasks: &[PluginTask],
    previews: &HashMap<String, InstallPreview>,
    has_chat: &impl Fn(&str) -> bool,
    rows: &mut Vec<BotInstallRow>,
    covered: &mut HashSet<&'a str>,
) {
    for plugin in &installed.plugins {
        if plugin.chat.is_none() {
            continue;
        }
        if let Some(failure) = &plugin.install_failure {
            if tasks
                .iter()
                .any(|task| task.id == plugin.id && task.state.in_progress())
            {
                continue;
            }
            rows.push(BotInstallRow {
                plugin_id: plugin.id.clone(),
                title: plugin.title.clone(),
                icon: plugin.icon.clone(),
                operation: PluginOperation::Install,
                source: plugin.source.clone(),
                state: BotInstallState::Failed {
                    reason: crate::plugins::api::failure_kind(Some(&failure.kind), 500),
                    message: match &plugin.status {
                        PluginState::Degraded { reason } => reason.clone(),
                        _ => failure.message.clone(),
                    },
                },
            });
            covered.insert(plugin.id.as_str());
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
            source: source(previews, &plugin.id).or_else(|| plugin.source.clone()),
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
        if !task.state.in_progress()
            && installed.is_some_and(|installed| {
                installed
                    .plugins
                    .iter()
                    .any(|plugin| plugin.id == task.id && plugin.install_failure.is_some())
            })
        {
            settling.remove(&task.id);
            continue;
        }
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
                // 已安装列表里还看不到它时，任务里家里服务器报到的阶段就是唯一线索。
                if covered.contains(task.id.as_str()) {
                    continue;
                }
                BotInstallState::Running {
                    progress: task.install_progress.clone().unwrap_or(InstallProgress {
                        stage: InstallStage::Other,
                        percent: None,
                    }),
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
            source: source(previews, &task.id).or_else(|| {
                installed
                    .and_then(|list| list.plugins.iter().find(|plugin| plugin.id == task.id))
                    .and_then(|plugin| plugin.source.clone())
            }),
            state,
        });
    }
}

fn source(previews: &HashMap<String, InstallPreview>, id: &str) -> Option<String> {
    previews.get(id).and_then(|preview| preview.source.clone())
}

#[cfg(test)]
mod tests;
