//! 安装、升级、卸载：一个插件同时只有一个任务，进展放在 `PluginsViewModel.tasks` 里。家里的服务器单独执行这些操作，请求
//! 断开了也照样做完（ADR-0017），所以连接断了不算失败，要从已安装列表里等结果。

use std::collections::HashSet;
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use tokio::sync::watch;

use wonton_l10n::l10n;
use wonton_protocol::{
    CapabilityProviderRef, CoreError, HomeStatus, HomeViewModel, InstallPreview, InstallRequest,
    InstallTogether, MissingCapability, PluginDetail, PluginFailure, PluginIcon, PluginOperation,
    PluginProblem, PluginState, PluginTask, PluginTaskNextStep, PluginTaskState, PluginsViewModel,
    SecretString, SecretUpdate, SecretValues, SelectedProvider, SettingValues, UninstallRequest,
    UpgradeRequest, UpgradeResult,
};

use super::api::{CoreApi, CoreApiError, INSTALL_TIMEOUT, to_core_error};
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
    /// 主人在预览里选中的设备 ID（ADR-0036）；没选为空，升级时为空就沿用装着的。可选权限关着的那一类不带。
    fn chosen_devices(preview: &InstallPreview) -> Vec<String> {
        crate::chosen_device_ids(
            preview.devices.clone().unwrap_or_default(),
            preview.permissions.clone(),
        )
    }

    /// 安装失败回滚后，核心保留原始安装决定时直接重装，不重新请求来源或权限确认。
    pub(crate) fn retry_install(self: &Arc<Self>, id: String) -> Result<(), CoreError> {
        let (title, icon) = Self::retry_target(&self.view.borrow(), &id)?;
        self.begin_task(&id, &title, icon, PluginOperation::Install)?;
        let link = self.clone();
        let generation = self.generation.load(Ordering::SeqCst);
        self.runtime.spawn(async move {
            let Some(api) = link.task_api(&id, generation) else {
                return;
            };
            let path = ["plugins", id.as_str(), "retry"];
            let body = serde_json::json!({});
            let sent = api.post_unit(&path, &body, INSTALL_TIMEOUT);
            link.finish_task(&id, Outcome::Running { version: None }, sent)
                .await;
        });
        Ok(())
    }

    /// 能不能按保留的决定重试：结果不确定时不重试（可能已经在装），核心没保留决定的也不行。
    /// 能重试时给出任务行的标题和图标。UI 假核心也用它，和真实核心拒绝的情形一致。
    pub(crate) fn retry_target(
        view: &PluginsViewModel,
        id: &str,
    ) -> Result<(String, Option<PluginIcon>), CoreError> {
        if view
            .tasks
            .iter()
            .any(|task| task.id == id && matches!(task.state, PluginTaskState::Uncertain { .. }))
        {
            return Err(CoreError::NotReady {
                reason: l10n!(CATALOG, language(), "plugins-tasks-retry-unavailable"),
            });
        }
        let plugin = view
            .installed
            .as_ref()
            .and_then(|installed| installed.plugins.iter().find(|plugin| plugin.id == id))
            .ok_or_else(|| CoreError::InvalidInput {
                reason: l10n!(CATALOG, language(), "plugins-tasks-no-such-task", id = id),
            })?;
        if !plugin
            .install_failure
            .as_ref()
            .is_some_and(|failure| failure.retryable)
        {
            return Err(CoreError::NotReady {
                reason: l10n!(CATALOG, language(), "plugins-tasks-retry-unavailable"),
            });
        }
        Ok((plugin.title.clone(), plugin.icon.clone()))
    }

    /// `InstallPlugin`：按预览安装，授予预览里列出的全部权限。
    pub(crate) fn install(
        self: &Arc<Self>,
        mut preview: InstallPreview,
        secrets: SecretValues,
        settings: SettingValues,
    ) -> Result<(), CoreError> {
        let grant = confirmed(&preview)?;
        self.resolve_confirmation(&preview);
        self.remember_preview(&preview);
        let lan_webui = lan_webui_chosen(&preview);
        let enabled_permissions = enabled_permissions(&preview);
        let audience = preview
            .audience
            .take()
            .and_then(crate::selected_plugin_audience);
        let devices = Self::chosen_devices(&preview);
        let missing = preview.missing_requires.clone();
        // 预览里每个缺的必需能力都带着选中的提供方；一起装的请求按预览原样带上（ADR-0053）。
        let decisions = install_together(&preview);
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
        let mut request = InstallRequest {
            source,
            digest: preview.digest,
            plan: preview.plan,
            scope: None,
            audience,
            grant,
            enabled_permissions,
            secrets,
            settings,
            devices,
            lan_webui,
            install_together: Vec::new(),
            // 主人勾选的高权限使用方（ADR-0053 第 3 条）；提供方内层一律为空。
            connect_plugins: preview.connected_plugins.clone(),
            remote_oauth: None,
        };
        let link = self.clone();
        let id = preview.id;
        let generation = self.generation.load(Ordering::SeqCst);
        self.runtime.spawn(async move {
            let Some(api) = link.task_api(&id, generation) else {
                return;
            };
            let sent = async {
                // 新预览带着每个提供方的完整决定；老预览（聊天导入、旧核心）退回按来源取第一个。
                request.install_together = match decisions {
                    Ok(together) if !together.is_empty() => together,
                    Ok(_) => link.providers(&missing).await.map_err(problem)?,
                    Err(error) => return Err(problem(error)),
                };

                api.post_unit(&["plugins"], &request, INSTALL_TIMEOUT).await
            };
            link.finish_task(&id, Outcome::Running { version: None }, sent)
                .await;
        });
        Ok(())
    }

    /// 一起装的提供方（ADR-0053）：预览里缺的必需能力各取列表的第一个提供方，连同它自己的预览一起装。
    /// 老预览（聊天导入、旧核心）没有提供方的完整决定时，按来源取列表的第一个再装。
    async fn providers(
        self: &Arc<Self>,
        missing: &[MissingCapability],
    ) -> Result<Vec<InstallTogether>, CoreError> {
        let mut together = Vec::with_capacity(missing.len());
        let mut chosen = HashSet::new();
        for capability in missing {
            let provider = capability
                .providers
                .first()
                .ok_or_else(|| CoreError::NotReady {
                    reason: l10n!(
                        CATALOG,
                        language(),
                        "plugins-tasks-preview-no-provider",
                        capability = capability.capability.as_str()
                    ),
                })?;
            // 同一个提供方同时补几个缺的能力时只装一次：核心的预检也只收一条，装上它几个能力都补齐。
            if !chosen.insert(provider.id.as_str()) {
                continue;
            }
            let preview = self.preview_install(provider.source.clone()).await?;
            together.push(provider_install(&SelectedProvider {
                provider: provider.clone(),
                preview,
                secrets: Default::default(),
            })?);
        }
        Ok(together)
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
            .map(super::identity::detail)
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
            devices: Self::chosen_devices(&preview),
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

    /// `UninstallPlugin`：标题、图标和分类从已安装列表里取。先卸 `dependents`（保留数据），再卸这个；
    /// `providers` 是同一次请求里一起卸的、没人再用的提供方（ADR-0053）。被在用的依赖挡住时把依赖方
    /// 留在任务上，壳据此给出“一起卸载”或“先停它们”。
    pub(crate) fn uninstall(
        self: &Arc<Self>,
        id: String,
        delete_data: bool,
        providers: Vec<String>,
        dependents: Vec<String>,
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
            let sent = async {
                for dependent in &dependents {
                    let deleted = api
                        .delete_detailed(
                            &["plugins", dependent.as_str()],
                            &[],
                            None::<&UninstallRequest>,
                            INSTALL_TIMEOUT,
                        )
                        .await;
                    if let Err(error) = deleted {
                        return Err(match error {
                            CoreApiError::Http {
                                message, plugins, ..
                            } if !plugins.is_empty() => PluginProblem {
                                reason: PluginFailure::InUse,
                                message: l10n!(
                                    CATALOG,
                                    language(),
                                    "plugins-tasks-dependent-in-use",
                                    id = dependent.as_str(),
                                    detail = message
                                ),
                                min_version: None,
                                settings: None,
                            },
                            error => error.into_problem(),
                        });
                    }
                }
                let query: &[(&str, &str)] = if delete_data {
                    &[("purge", "true")]
                } else {
                    &[]
                };
                let path = ["plugins", id.as_str()];
                let request = (!providers.is_empty()).then(|| UninstallRequest {
                    providers: providers.clone(),
                });
                let body = request.as_ref();
                match api
                    .delete_detailed(&path, query, body, INSTALL_TIMEOUT)
                    .await
                {
                    Ok(()) => Ok(()),
                    Err(CoreApiError::Http {
                        message, plugins, ..
                    }) if !plugins.is_empty() => {
                        link.set_task_dependents(&id, plugins);
                        Err(PluginProblem {
                            reason: PluginFailure::InUse,
                            message,
                            min_version: None,
                            settings: None,
                        })
                    }
                    Err(error) => Err(error.into_problem()),
                }
            };
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
    pub(super) fn remember_preview(&self, preview: &InstallPreview) {
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
            outcome = Self::push_task(view, id, title, icon, operation);
            outcome.is_ok()
        });
        if outcome.is_ok() {
            self.start_polling();
        }
        outcome
    }

    /// 任务行开始转：同一个插件已有在做的任务时拒绝，否则换掉它的旧任务行。UI 假核心也用它。
    pub(crate) fn push_task(
        view: &mut PluginsViewModel,
        id: &str,
        title: &str,
        icon: Option<PluginIcon>,
        operation: PluginOperation,
    ) -> Result<(), CoreError> {
        if view
            .tasks
            .iter()
            .any(|task| task.id == id && task.state.in_progress())
        {
            return Err(CoreError::NotReady {
                reason: l10n!(
                    CATALOG,
                    language(),
                    "plugins-tasks-task-running",
                    title = title
                ),
            });
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
            dependents: Vec::new(),
        });
        Self::derive(view);
        Ok(())
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

    /// 记下挡住这次卸载的依赖方：任务停在失败上，壳照它列出名字。
    fn set_task_dependents(&self, id: &str, dependents: Vec<CapabilityProviderRef>) {
        self.view.send_modify(|view| {
            if let Some(task) = view.tasks.iter_mut().find(|task| task.id == id) {
                task.dependents = dependents;
            }
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

/// 主人在确认页上打开了开关（[`PluginAction::InstallPlugin`](wonton_protocol::PluginAction::InstallPlugin)）。
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

/// 预览里选中的提供方转成一起装的请求（ADR-0053）：同一个提供方补几个缺的能力只装一次。
/// 缺的必需能力没有选中提供方时给出“商店里没有提供方”。
fn install_together(preview: &InstallPreview) -> Result<Vec<InstallTogether>, CoreError> {
    if preview.install_providers.is_empty() {
        return Ok(Vec::new());
    }
    let mut together: Vec<InstallTogether> = Vec::new();
    for section in &preview.install_providers {
        let Some(selected) = section.selected.as_ref() else {
            return Err(CoreError::NotReady {
                reason: l10n!(
                    CATALOG,
                    language(),
                    "plugins-tasks-preview-no-provider",
                    capability = section.capability.as_str()
                ),
            });
        };
        // 同一个提供方补几个能力只装一次；几节里填的密钥合到这一份决定上。
        if let Some(existing) = together
            .iter_mut()
            .find(|together| together.id == selected.provider.id)
        {
            existing.request.secrets.extend(selected.secrets.clone());
            continue;
        }
        together.push(provider_install(selected)?);
    }
    Ok(together)
}

/// 一个提供方的安装决定：它预览里列出的权限照授、主人填的密钥带上，内层不再带别的依赖（ADR-0053）。
fn provider_install(selected: &SelectedProvider) -> Result<InstallTogether, CoreError> {
    let mut preview = selected.preview.clone();
    if let Some(blocked) = preview.blocked.clone().or(preview.missing_profile.clone()) {
        return Err(CoreError::NotReady { reason: blocked });
    }
    let grant = confirmed(&preview)?;
    let enabled_permissions = enabled_permissions(&preview);
    let lan_webui = lan_webui_chosen(&preview);
    let devices = PluginsLink::chosen_devices(&preview);
    let audience = preview
        .audience
        .take()
        .and_then(crate::selected_plugin_audience);
    let Some(source) = preview.source.take() else {
        return Err(CoreError::InvalidInput {
            reason: l10n!(CATALOG, language(), "plugins-tasks-preview-no-source"),
        });
    };
    Ok(InstallTogether {
        id: selected.provider.id.clone(),
        request: InstallRequest {
            source,
            digest: preview.digest,
            plan: preview.plan,
            scope: None,
            audience,
            grant,
            enabled_permissions,
            secrets: selected.secrets.clone(),
            settings: Default::default(),
            devices,
            lan_webui,
            install_together: Vec::new(),
            connect_plugins: Vec::new(),
            remote_oauth: None,
        },
    })
}

/// 取提供方预览失败：断线照样可能已经装上了（从已安装列表等结果），错误按原因分给任务，不再
/// 混成“输入非法”。NotReady 是“提供方要单独确认/商店里没有提供方”，和 onboarding 的 core_problem
/// 一样归 Upstream，不给“重新安装”的下一步。
pub(super) fn problem(error: CoreError) -> PluginProblem {
    let (reason, message, min_version) = match error {
        CoreError::TransportUnavailable { reason } => {
            (PluginFailure::TransportUnavailable, reason, None)
        }
        CoreError::InvalidInput { reason } | CoreError::SettingsInvalid { reason, .. } => {
            (PluginFailure::Invalid, reason, None)
        }
        CoreError::Unauthorized { reason } => (PluginFailure::Unauthorized, reason, None),
        CoreError::Forbidden { reason }
        | CoreError::Storage { reason }
        | CoreError::NotReady { reason }
        | CoreError::Upstream { reason } => (PluginFailure::Upstream, reason, None),
        CoreError::ServerTooOld {
            reason,
            min_version,
        } => (PluginFailure::ServerTooOld, reason, min_version),
    };
    PluginProblem {
        reason,
        message,
        min_version,
        settings: None,
    }
}
