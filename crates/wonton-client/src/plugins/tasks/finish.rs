//! 任务的结果落定：请求返回、断线后从已安装列表等结果、等不到结果时怎么定性。

use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use wonton_l10n::l10n;
use wonton_protocol::{
    PluginFailure, PluginProblem, PluginState, PluginTaskNextStep, PluginTaskState,
};

use super::api::INSTALL_TIMEOUT;
use super::{POLL_INTERVAL, PluginsLink};
use crate::l10n::{CATALOG, language};

/// 连接断了以后，最多再等家里的服务器这么久：和安装请求自己的上限一样。
const SETTLE_TIMEOUT: Duration = INSTALL_TIMEOUT;
/// 连续这么久都问不到家里的服务器，就不再等了：任务转着的时候主人既不能重试，也关不掉它。
const UNREACHABLE_LIMIT: Duration = Duration::from_secs(120);
/// 连续这么多次在已安装列表里看不到插件，才算请求没到家里的服务器。发起之后它可能还没出现在列表里。
const MISSING_LIMIT: u32 = 3;

/// 任务做完后已安装列表应该是什么样。
pub(in crate::plugins) enum Outcome {
    /// 插件在运行；升级时还要是新版本。
    Running {
        version: Option<String>,
    },
    Removed,
}

impl PluginsLink {
    pub(in crate::plugins) async fn finish_task(
        self: &Arc<Self>,
        id: &str,
        outcome: Outcome,
        sent: impl Future<Output = Result<(), PluginProblem>>,
    ) {
        let generation = self.generation.load(Ordering::SeqCst);
        // 请求要等家里的服务器做完才返回，断网时可能吊很久：连接一离开、回来就先后改状态，不等请求报错。
        let mut home = self.home.subscribe();
        let mut waiting = false;
        home.borrow_and_update();
        self.refresh_waiting(id, generation, &mut waiting, &home);
        let mut sent = std::pin::pin!(sent);
        let mut home_live = true;
        let sent = loop {
            tokio::select! {
                result = &mut sent => break result,
                changed = home.changed(), if home_live => {
                    if changed.is_err() {
                        home_live = false;
                        continue;
                    }
                    self.refresh_waiting(id, generation, &mut waiting, &home);
                }
            }
        };
        let state = match sent {
            Ok(()) => PluginTaskState::Succeeded,
            Err(problem) if problem.reason == PluginFailure::TransportUnavailable => {
                self.set_waiting(id, true, generation);
                self.settle(id, &outcome, generation).await
            }
            Err(problem) => PluginTaskState::Failed {
                reason: problem.reason,
                message: problem.message,
            },
        };
        // 换了服务器时任务已经标成了不确定，同名的新任务也不是这一个。
        if !self.is_current(generation) {
            return;
        }
        self.set_task_state(id, state);
        self.refresh_installed();
        // 商店每一行的“获取”“打开”看的是核心算好的已安装版本，装完也要重取。
        if self.store_wanted.load(Ordering::Relaxed) {
            self.refresh_store(false);
        }
    }

    /// 连接断了：家里的服务器可能还在做，从已安装列表里等结果。等的时候不说成在做，也不重试请求。
    async fn settle(&self, id: &str, outcome: &Outcome, generation: u64) -> PluginTaskState {
        let deadline = Instant::now() + SETTLE_TIMEOUT;
        let mut answered = Instant::now();
        let mut missing = 0;
        // 进来时刚标成在等连接；连上了先回到“在做”，又断了再标回去。
        let mut waiting = true;
        let mut home = self.home.subscribe();
        let mut home_live = true;
        // 连接状态可能比轮询还频繁：等下一次取列表的时间要跨迭代保留，否则一直重睡，取列表轮不到。
        let mut next_poll = Instant::now() + POLL_INTERVAL;
        while Instant::now() < deadline {
            let until = next_poll.saturating_duration_since(Instant::now());
            tokio::select! {
                () = tokio::time::sleep(until) => {
                    next_poll = Instant::now() + POLL_INTERVAL;
                }
                changed = home.changed(), if home_live => {
                    if changed.is_err() {
                        home_live = false;
                    } else {
                        self.refresh_waiting(id, generation, &mut waiting, &home);
                    }
                    continue;
                }
            }
            let result = self.list_installed().await;
            // 新服务器的列表里没有这个插件，不能算卸载成功。
            if !self.is_current(generation) {
                return server_changed();
            }
            let installed = match result {
                Ok(installed) => {
                    answered = Instant::now();
                    if waiting {
                        self.set_waiting(id, false, generation);
                        waiting = false;
                    }
                    installed
                }
                Err(problem) if problem.reason == PluginFailure::TransportUnavailable => {
                    if !waiting {
                        self.set_waiting(id, true, generation);
                        waiting = true;
                    }
                    if answered.elapsed() >= UNREACHABLE_LIMIT {
                        return uncertain();
                    }
                    continue;
                }
                // 问不到列表，不等于操作失败了。
                Err(problem) => {
                    return PluginTaskState::Uncertain {
                        message: problem.message,
                        next_step: PluginTaskNextStep::CheckInstalled,
                    };
                }
            };
            let plugin = installed.plugins.iter().find(|plugin| plugin.id == id);
            match (outcome, plugin) {
                (Outcome::Removed, None) => return PluginTaskState::Succeeded,
                (Outcome::Running { .. }, None) => {
                    missing += 1;
                    if missing >= MISSING_LIMIT {
                        return uncertain();
                    }
                }
                (_, Some(plugin)) => {
                    missing = 0;
                    if let Some(state) = settled_state(plugin, outcome) {
                        return state;
                    }
                }
            }
        }
        uncertain()
    }
}

fn settled_state(
    plugin: &wonton_protocol::InstalledPlugin,
    outcome: &Outcome,
) -> Option<PluginTaskState> {
    if let Some(state) = install_failure_state(plugin, outcome) {
        return Some(state);
    }
    match &plugin.status {
        PluginState::Installing | PluginState::Upgrading | PluginState::Uninstalling => None,
        PluginState::Degraded { reason } => Some(match outcome {
            // 卸载和升级前它可能就是坏的，请求还没到时也是这样，只有安装能据此算失败。
            Outcome::Running { version: None } => PluginTaskState::Failed {
                reason: PluginFailure::Upstream,
                message: reason.clone(),
            },
            _ => uncertain(),
        }),
        PluginState::Stopped | PluginState::Unknown => Some(uncertain()),
        PluginState::Running { .. } | PluginState::NeedsConfiguration => Some(match outcome {
            Outcome::Running { version }
                if version
                    .as_ref()
                    .is_none_or(|version| *version == plugin.version) =>
            {
                PluginTaskState::Succeeded
            }
            _ => uncertain(),
        }),
    }
}

fn install_failure_state(
    plugin: &wonton_protocol::InstalledPlugin,
    outcome: &Outcome,
) -> Option<PluginTaskState> {
    let failure = plugin.install_failure.as_ref()?;
    Some(match outcome {
        Outcome::Running { version: None } => PluginTaskState::Failed {
            reason: super::api::failure_kind(Some(&failure.kind), 500),
            message: match &plugin.status {
                PluginState::Degraded { reason } => reason.clone(),
                _ => failure.message.clone(),
            },
        },
        _ => uncertain(),
    })
}

fn uncertain() -> PluginTaskState {
    PluginTaskState::Uncertain {
        message: l10n!(CATALOG, language(), "plugins-tasks-uncertain"),
        next_step: PluginTaskNextStep::CheckInstalled,
    }
}

pub(in crate::plugins) fn server_changed() -> PluginTaskState {
    PluginTaskState::Uncertain {
        message: l10n!(CATALOG, language(), "plugins-tasks-server-changed"),
        next_step: PluginTaskNextStep::CheckInstalled,
    }
}
