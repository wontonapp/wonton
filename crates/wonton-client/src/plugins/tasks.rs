//! 安装、升级、卸载：一个插件同时只有一个任务，进展放在 `PluginsViewModel.tasks` 里。家里的服务器单独执行这些操作，请求
//! 断开了也照样做完（ADR-0017），所以连接断了不算失败，要从已安装列表里等结果。

use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use wonton_protocol::{
    CoreError, InstallPreview, InstallRequest, PluginFailure, PluginIcon, PluginOperation,
    PluginProblem, PluginState, PluginTask, PluginTaskState, UpgradeRequest, UpgradeResult,
};

use super::api::{CoreApi, INSTALL_TIMEOUT};
use super::{POLL_INTERVAL, PluginsLink};

/// 连接断了以后，最多再等家里的服务器这么久：和安装请求自己的上限一样。
const SETTLE_TIMEOUT: Duration = INSTALL_TIMEOUT;
/// 连续这么久都问不到家里的服务器，就不再等了：任务转着的时候主人既不能重试，也关不掉它。
const UNREACHABLE_LIMIT: Duration = Duration::from_secs(120);
/// 连续这么多次在已安装列表里看不到插件，才算请求没到家里的服务器。发起之后它可能还没出现在列表里。
const MISSING_LIMIT: u32 = 3;
/// 等不到结果的说明。
const UNCERTAIN: &str = "连接断了，等不到家里服务器的结果，看一下已安装列表";
const SERVER_CHANGED: &str = "换了家里的服务器，等不到原来那台的结果";

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
    pub(crate) fn install(self: &Arc<Self>, preview: InstallPreview) -> Result<(), CoreError> {
        let grant = confirmed(&preview)?;
        let lan_webui = lan_webui_chosen(&preview);
        let Some(source) = preview.source else {
            return Err(CoreError::InvalidInput {
                reason: "这份预览没有来源，装不了".to_owned(),
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
            grant,
            devices: Vec::new(),
            lan_webui,
        };
        let link = self.clone();
        let id = preview.id;
        self.runtime.spawn(async move {
            let Some(api) = link.task_api(&id) else {
                return;
            };
            let sent = api.post_unit(&["plugins"], &request, INSTALL_TIMEOUT);
            link.finish_task(&id, Outcome::Running { version: None }, sent)
                .await;
        });
        Ok(())
    }

    /// `UpgradePlugin`：升到预览给的版本。
    pub(crate) fn upgrade(self: &Arc<Self>, preview: InstallPreview) -> Result<(), CoreError> {
        let grant = confirmed(&preview)?;
        let lan_webui = lan_webui_chosen(&preview);
        self.begin_task(
            &preview.id,
            &preview.title,
            preview.icon.clone(),
            PluginOperation::Upgrade,
        )?;
        let request = UpgradeRequest {
            devices: Vec::new(),
            source: preview.source,
            digest: preview.digest,
            grant,
            lan_webui,
        };
        let link = self.clone();
        let id = preview.id;
        let version = preview.version;
        self.runtime.spawn(async move {
            let Some(api) = link.task_api(&id) else {
                return;
            };
            let path = ["plugins", id.as_str(), "upgrade"];
            let generation = link.generation.load(Ordering::SeqCst);
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
        self.runtime.spawn(async move {
            let Some(api) = link.task_api(&id) else {
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

    /// `DismissPluginTask`：清掉做完的任务；没做完的不能清，壳不会给这个按钮。
    pub(crate) fn dismiss_task(&self, id: &str) -> Result<(), CoreError> {
        let mut outcome = Ok(());
        self.view.send_if_modified(|view| {
            let Some(index) = view.tasks.iter().position(|task| task.id == id) else {
                outcome = Err(CoreError::InvalidInput {
                    reason: format!("没有这个任务：{id}"),
                });
                return false;
            };
            if view.tasks[index].state == PluginTaskState::Running {
                outcome = Err(CoreError::NotReady {
                    reason: "任务还没做完".to_owned(),
                });
                return false;
            }
            view.tasks.remove(index);
            true
        });
        outcome
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
                .any(|task| task.id == id && task.state == PluginTaskState::Running)
            {
                outcome = Err(CoreError::NotReady {
                    reason: format!("{title} 正在安装、升级或卸载"),
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
                added_tools: None,
            });
            true
        });
        if outcome.is_ok() {
            self.start_polling();
        }
        outcome
    }

    /// 连接信息都没有时，请求肯定没发出去：结果是确定的，不用等。
    fn task_api(&self, id: &str) -> Option<CoreApi> {
        match self.api() {
            Ok(api) => Some(api),
            Err(problem) => {
                self.set_task_state(
                    id,
                    PluginTaskState::Failed {
                        reason: problem.reason,
                        message: problem.message,
                    },
                );
                None
            }
        }
    }

    fn set_task_state(&self, id: &str, state: PluginTaskState) {
        self.view.send_modify(|view| {
            if let Some(task) = view.tasks.iter_mut().find(|task| task.id == id) {
                task.state = state;
            }
        });
    }

    /// 升级已经成功了，响应读不出来不算失败：核心比 App 新时形状可以变，这时不显示新增的工具。
    fn set_added_tools(&self, id: &str, response: &[u8]) {
        let added = match serde_json::from_slice::<UpgradeResult>(response) {
            Ok(result) => result.added_tools,
            Err(error) => {
                tracing::warn!("升级的响应读不出来：{error}");
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
        let state = match sent.await {
            Ok(()) => PluginTaskState::Succeeded,
            Err(problem) if problem.reason == PluginFailure::TransportUnavailable => {
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

    /// 连接断了：家里的服务器可能还在做，从已安装列表里等结果。
    async fn settle(&self, id: &str, outcome: &Outcome, generation: u64) -> PluginTaskState {
        let deadline = Instant::now() + SETTLE_TIMEOUT;
        let mut answered = Instant::now();
        let mut missing = 0;
        while Instant::now() < deadline {
            tokio::time::sleep(POLL_INTERVAL).await;
            let result = self.list_installed().await;
            // 新服务器的列表里没有这个插件，不能算卸载成功。
            if !self.is_current(generation) {
                return server_changed();
            }
            let installed = match result {
                Ok(installed) => {
                    answered = Instant::now();
                    installed
                }
                Err(problem) if problem.reason == PluginFailure::TransportUnavailable => {
                    if answered.elapsed() >= UNREACHABLE_LIMIT {
                        return uncertain();
                    }
                    continue;
                }
                // 问不到列表，不等于操作失败了。
                Err(problem) => {
                    return PluginTaskState::Uncertain {
                        message: problem.message,
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
                    match &plugin.status {
                        PluginState::Installing
                        | PluginState::Upgrading
                        | PluginState::Uninstalling => {}
                        PluginState::Degraded { reason } => {
                            // 卸载和升级前它可能就是坏的，请求还没到时也是这样，只有安装能据此算失败。
                            return match outcome {
                                Outcome::Running { version: None } => PluginTaskState::Failed {
                                    reason: PluginFailure::Upstream,
                                    message: reason.clone(),
                                },
                                _ => uncertain(),
                            };
                        }
                        // 容器停止不能证明操作已完成；未知状态也不能算成功。
                        PluginState::Stopped | PluginState::Unknown => return uncertain(),
                        PluginState::Running { .. } => match outcome {
                            Outcome::Running { version } => {
                                let done = version
                                    .as_ref()
                                    .is_none_or(|version| *version == plugin.version);
                                return if done {
                                    PluginTaskState::Succeeded
                                } else {
                                    uncertain()
                                };
                            }
                            Outcome::Removed => return uncertain(),
                        },
                    }
                }
            }
        }
        uncertain()
    }
}

fn uncertain() -> PluginTaskState {
    PluginTaskState::Uncertain {
        message: UNCERTAIN.to_owned(),
    }
}

pub(super) fn server_changed() -> PluginTaskState {
    PluginTaskState::Uncertain {
        message: SERVER_CHANGED.to_owned(),
    }
}

/// 主人在确认页上打开了开关（[`Action::InstallPlugin`](wonton_protocol::Action::InstallPlugin)）。
fn lan_webui_chosen(preview: &InstallPreview) -> bool {
    preview
        .lan_webui
        .as_ref()
        .is_some_and(|offer| offer.enabled)
}

/// 确认页上看到的就是授予的：预览里列出的全部权限（ADR-0017）。预览说装不了时不发请求。
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
        .map(|line| line.permission.clone())
        .collect())
}
