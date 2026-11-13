//! 安装、升级、卸载：一个插件同时只有一个任务，进展放在 `PluginsViewModel.tasks` 里。家里的服务器单独执行这些操作，请求
//! 断开了也照样做完（ADR-0017），所以连接断了不算失败，要从已安装列表里等结果。

use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use tokio::sync::watch;

use wonton_l10n::l10n;
use wonton_protocol::{
    CoreError, HomeStatus, HomeViewModel, InstallPreview, InstallRequest, PluginDetail,
    PluginFailure, PluginIcon, PluginOperation, PluginProblem, PluginState, PluginTask,
    PluginTaskNextStep, PluginTaskState, PluginsViewModel, SecretString, SecretUpdate,
    SecretValues, SettingValues, UpgradeRequest, UpgradeResult,
};

use super::api::{CoreApi, INSTALL_TIMEOUT, to_core_error};
use super::{POLL_INTERVAL, PluginsLink};
use crate::l10n::{CATALOG, language};

/// 连接断了以后，最多再等家里的服务器这么久：和安装请求自己的上限一样。
const SETTLE_TIMEOUT: Duration = INSTALL_TIMEOUT;
/// 连续这么久都问不到家里的服务器，就不再等了：任务转着的时候主人既不能重试，也关不掉它。
const UNREACHABLE_LIMIT: Duration = Duration::from_secs(120);
/// 连续这么多次在已安装列表里看不到插件，才算请求没到家里的服务器。发起之后它可能还没出现在列表里。
const MISSING_LIMIT: u32 = 3;

/// 任务做完后已安装列表应该是什么样。
enum Outcome {
    /// 插件在运行；升级时还要是新版本。
    Running {
        version: Option<String>,
    },
    Removed,
}

impl PluginsLink {
    /// `InstallPlugin`：按预览安装，授予预览里列出的全部权限。
    pub(crate) fn install(
        self: &Arc<Self>,
        preview: InstallPreview,
        secrets: SecretValues,
        settings: SettingValues,
    ) -> Result<(), CoreError> {
        let grant = confirmed(&preview)?;
        self.resolve_confirmation(&preview);
        self.remember_preview(&preview);
        let lan_webui = lan_webui_chosen(&preview);
        let enabled_permissions = enabled_permissions(&preview);
        let Some(source) = preview.source else {
            return Err(CoreError::InvalidInput {
                reason: l10n!(CATALOG, language(), "plugins-tasks-preview-no-source"),
            });
        };
        self.begin_task(
            &preview.id,
            &preview.title,
            preview.icon.clone(),
            PluginOperation::Install,
        )?;
        let request = InstallRequest {
            source,
            digest: preview.digest,
            plan: preview.plan,
            scope: None,
            grant,
            enabled_permissions,
            secrets,
            settings,
            devices: Vec::new(),
            lan_webui,
            install_together: Vec::new(),
            connect_plugins: Vec::new(),
        };
        let link = self.clone();
        let id = preview.id;
        let generation = self.generation.load(Ordering::SeqCst);
        self.runtime.spawn(async move {
            let Some(api) = link.task_api(&id, generation) else {
                return;
            };
            let sent = api.post_unit(&["plugins"], &request, INSTALL_TIMEOUT);
            link.finish_task(&id, Outcome::Running { version: None }, sent)
                .await;
        });
        Ok(())
    }

    /// 填一项密钥或清空可选项（ADR-0064）：核心重建用到它的服务，做完才回应。
    pub(crate) async fn set_secret_now(
        self: &Arc<Self>,
        id: String,
        name: String,
        value: Option<SecretString>,
    ) -> Result<PluginDetail, CoreError> {
        let api = self.api().map_err(to_core_error)?;
        let update = SecretUpdate { value };
        let result = api
            .put(
                &["plugins", &id, "secrets", &name],
                &update,
                INSTALL_TIMEOUT,
            )
            .await
            .map_err(to_core_error);
        self.refresh_installed();
        result
    }

    /// `UpgradePlugin`：升到预览给的版本。
    pub(crate) fn upgrade(
        self: &Arc<Self>,
        preview: InstallPreview,
        secrets: SecretValues,
        settings: SettingValues,
    ) -> Result<(), CoreError> {
        let grant = confirmed(&preview)?;
        self.resolve_confirmation(&preview);
        self.remember_preview(&preview);
        let lan_webui = lan_webui_chosen(&preview);
        self.begin_task(
            &preview.id,
            &preview.title,
            preview.icon.clone(),
            PluginOperation::Upgrade,
        )?;
        let enabled_permissions = enabled_permissions(&preview);
        let request = UpgradeRequest {
            devices: Vec::new(),
            source: preview.source,
            digest: preview.digest,
            grant,
            enabled_permissions,
            secrets,
            settings,
            lan_webui,
        };
        let link = self.clone();
        let id = preview.id;
        let version = preview.version;
        let generation = self.generation.load(Ordering::SeqCst);
        self.runtime.spawn(async move {
            let Some(api) = link.task_api(&id, generation) else {
                return;
            };
            let path = ["plugins", id.as_str(), "upgrade"];
            let sent = async {
                let bytes = api.post_raw(&path, &request, INSTALL_TIMEOUT).await?;
                if link.is_current(generation) {
                    link.set_added_tools(&id, &bytes);
                }
                Ok(())
            };
            let outcome = Outcome::Running {
                version: Some(version.clone()),
            };
            link.finish_task(&id, outcome, sent).await;
        });
        Ok(())
    }

    /// `UninstallPlugin`：标题、图标和分类从已安装列表里取。
    pub(crate) fn uninstall(
        self: &Arc<Self>,
        id: String,
        delete_data: bool,
    ) -> Result<(), CoreError> {
        let (title, icon) = {
            let view = self.view.borrow();
            let plugin = view
                .installed
                .as_ref()
                .and_then(|installed| installed.plugins.iter().find(|plugin| plugin.id == id));
            plugin.map_or_else(
                || (id.clone(), None),
                |plugin| (plugin.title.clone(), plugin.icon.clone()),
            )
        };
        self.begin_task(&id, &title, icon, PluginOperation::Uninstall)?;
        let link = self.clone();
        let generation = self.generation.load(Ordering::SeqCst);
        self.runtime.spawn(async move {
            let Some(api) = link.task_api(&id, generation) else {
                return;
            };
            let query: &[(&str, &str)] = if delete_data {
                &[("purge", "true")]
            } else {
                &[]
            };
            let path = ["plugins", id.as_str()];
            let sent = api.delete(&path, query, INSTALL_TIMEOUT);
            link.finish_task(&id, Outcome::Removed, sent).await;
        });
        Ok(())
    }

    /// 家里的服务器报到的阶段（拉镜像、准备、启动、等 Bot）放进在做的任务：升级几分钟里只有小圈时，
    /// 主人看不出在做哪一步。跟着已安装列表走，列表里不忙了就清掉，任务自己的结果另说。
    pub(super) fn sync_task_progress(view: &mut PluginsViewModel) -> bool {
        let busy = |status: &PluginState| {
            matches!(
                status,
                PluginState::Installing | PluginState::Upgrading | PluginState::Uninstalling
            )
        };
        let mut changed = false;
        for task in &mut view.tasks {
            if !task.state.in_progress() || task.operation == PluginOperation::Uninstall {
                if task.install_progress.is_some() {
                    task.install_progress = None;
                    changed = true;
                }
                continue;
            }
            let progress = view
                .installed
                .as_ref()
                .and_then(|installed| installed.plugins.iter().find(|plugin| plugin.id == task.id))
                .filter(|plugin| busy(&plugin.status))
                .and_then(|plugin| plugin.install_progress.clone());
            if task.install_progress != progress {
                task.install_progress = progress;
                changed = true;
            }
        }
        changed
    }

    /// `DismissPluginTask`：清掉做完的任务；没做完的不能清，壳不会给这个按钮。
    pub(crate) fn dismiss_task(&self, id: &str) -> Result<(), CoreError> {
        let mut outcome = Ok(());
        self.view.send_if_modified(|view| {
            let Some(index) = view.tasks.iter().position(|task| task.id == id) else {
                outcome = Err(CoreError::InvalidInput {
                    reason: l10n!(CATALOG, language(), "plugins-tasks-no-such-task", id = id),
                });
                return false;
            };
            if view.tasks[index].state.in_progress() {
                outcome = Err(CoreError::NotReady {
                    reason: l10n!(CATALOG, language(), "plugins-tasks-task-not-done"),
                });
                return false;
            }
            view.tasks.remove(index);
            true
        });
        if outcome.is_ok() {
            self.previews
                .lock()
                .expect("the previews lock never panics")
                .remove(id);
        }
        outcome
    }

    /// 记下预览：失败后重试要来源，聊天列表的安装行要知道它是不是 Bot 插件。
    fn remember_preview(&self, preview: &InstallPreview) {
        self.previews
            .lock()
            .expect("the previews lock never panics")
            .insert(preview.id.clone(), preview.clone());
    }

    fn begin_task(
        self: &Arc<Self>,
        id: &str,
        title: &str,
        icon: Option<PluginIcon>,
        operation: PluginOperation,
    ) -> Result<(), CoreError> {
        let mut outcome = Ok(());
        self.view.send_if_modified(|view| {
            if view
                .tasks
                .iter()
                .any(|task| task.id == id && task.state.in_progress())
            {
                outcome = Err(CoreError::NotReady {
                    reason: l10n!(
                        CATALOG,
                        language(),
                        "plugins-tasks-task-running",
                        title = title
                    ),
                });
                return false;
            }
            view.tasks.retain(|task| task.id != id);
            view.tasks.push(PluginTask {
                id: id.to_owned(),
                title: title.to_owned(),
                icon,
                operation,
                state: PluginTaskState::Running,
                install_progress: None,
                waiting_for_connection: false,
                added_tools: None,
            });
            Self::derive(view);
            true
        });
        if outcome.is_ok() {
            self.start_polling();
        }
        outcome
    }

    /// 连接信息都没有时，请求发不出去：结果是确定的，不用等。
    /// 换过服务器就不同了：请求本来会发到上一台，结果不知道，`forget_server` 也已经把在做的任务标成
    /// 不确定，这里再盖一次而不是写“连不上”，否则配对时先到的失败会把结果说成确定的。
    fn task_api(&self, id: &str, generation: u64) -> Option<CoreApi> {
        match self.api() {
            Ok(api) => Some(api),
            Err(problem) => {
                let state = if self.is_current(generation) {
                    PluginTaskState::Failed {
                        reason: problem.reason,
                        message: problem.message,
                    }
                } else {
                    server_changed()
                };
                self.set_task_state(id, state);
                None
            }
        }
    }

    fn set_task_state(&self, id: &str, state: PluginTaskState) {
        self.view.send_modify(|view| {
            if let Some(task) = view.tasks.iter_mut().find(|task| task.id == id) {
                task.state = state;
                task.waiting_for_connection = false;
            }
            Self::derive(view);
        });
    }

    /// 连接状态变了：跟着改“在等连接”。`waiting` 是本地的上一份状态，只在变化时写一次。
    fn refresh_waiting(
        &self,
        id: &str,
        generation: u64,
        waiting: &mut bool,
        home: &watch::Receiver<HomeViewModel>,
    ) {
        let now_waiting = home.borrow().status != HomeStatus::Connected;
        if now_waiting != *waiting {
            self.set_waiting(id, now_waiting, generation);
            *waiting = now_waiting;
        }
    }

    /// 连接断了或者回来了：界面要能说出在等连接，而不是一直像在做。换了服务器以后不再动上一台的任务。
    fn set_waiting(&self, id: &str, waiting: bool, generation: u64) {
        self.view.send_if_modified(|view| {
            if !self.is_current(generation) {
                return false;
            }
            let Some(task) = view.tasks.iter_mut().find(|task| task.id == id) else {
                return false;
            };
            if task.waiting_for_connection == waiting {
                return false;
            }
            task.waiting_for_connection = waiting;
            Self::derive(view);
            true
        });
    }

    /// 升级已经成功了，响应读不出来不算失败：核心比 App 新时形状可以变，这时不显示新增的工具。
    fn set_added_tools(&self, id: &str, response: &[u8]) {
        let added = match serde_json::from_slice::<UpgradeResult>(response) {
            Ok(result) => result.added_tools,
            Err(error) => {
                tracing::warn!("could not read the upgrade response: {error}");
                return;
            }
        };
        self.view.send_modify(|view| {
            if let Some(task) = view.tasks.iter_mut().find(|task| task.id == id) {
                task.added_tools = Some(added);
            }
        });
    }

    async fn finish_task(
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

pub(super) fn server_changed() -> PluginTaskState {
    PluginTaskState::Uncertain {
        message: l10n!(CATALOG, language(), "plugins-tasks-server-changed"),
        next_step: PluginTaskNextStep::CheckInstalled,
    }
}

/// 主人在确认页上打开了开关（[`Action::InstallPlugin`](wonton_protocol::Action::InstallPlugin)）。
fn lan_webui_chosen(preview: &InstallPreview) -> bool {
    preview
        .lan_webui
        .as_ref()
        .is_some_and(|offer| offer.enabled)
}

/// 确认页上看到的就是授予的：预览里列出的必需权限，可选项不在这里（ADR-0057）。预览说装不了时不发请求。
fn confirmed(preview: &InstallPreview) -> Result<Vec<String>, CoreError> {
    if let Some(blocked) = preview
        .blocked
        .as_ref()
        .or(preview.missing_profile.as_ref())
    {
        return Err(CoreError::NotReady {
            reason: blocked.clone(),
        });
    }
    Ok(preview
        .permissions
        .iter()
        .filter(|line| !line.optional)
        .map(|line| line.permission.clone())
        .collect())
}

/// 主人在确认页上打开了的可选项。
fn enabled_permissions(preview: &InstallPreview) -> Vec<String> {
    preview
        .permissions
        .iter()
        .filter(|line| line.optional && line.enabled)
        .map(|line| line.permission.clone())
        .collect()
}
