//! 任务行本身：开一条、报阶段、改状态、清掉，以及跟着连接状态变的“在等连接”。操作由 [`super::run`] 发起。

use std::sync::Arc;

use tokio::sync::watch;

use wonton_l10n::l10n;
use wonton_protocol::{
    CapabilityProviderRef, CoreError, HomeStatus, HomeViewModel, PluginIcon, PluginOperation,
    PluginState, PluginTask, PluginTaskState, PluginsViewModel, UpgradeResult,
};

use super::PluginsLink;
use super::api::CoreApi;
use super::finish::server_changed;
use crate::l10n::{CATALOG, language};

impl PluginsLink {
    /// 家里的服务器报到的阶段（拉镜像、准备、启动、等 Bot）放进在做的任务：升级几分钟里只有小圈时，
    /// 主人看不出在做哪一步。跟着已安装列表走，列表里不忙了就清掉，任务自己的结果另说。
    pub(in crate::plugins) fn sync_task_progress(view: &mut PluginsViewModel) -> bool {
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

    pub(in crate::plugins) fn begin_task(
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
    pub(in crate::plugins) fn task_api(&self, id: &str, generation: u64) -> Option<CoreApi> {
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

    pub(in crate::plugins) fn set_task_state(&self, id: &str, state: PluginTaskState) {
        self.view.send_modify(|view| {
            if let Some(task) = view.tasks.iter_mut().find(|task| task.id == id) {
                task.state = state;
                task.waiting_for_connection = false;
            }
            Self::derive(view);
        });
    }

    /// 记下挡住这次卸载的依赖方：任务停在失败上，壳照它列出名字。
    pub(in crate::plugins) fn set_task_dependents(
        &self,
        id: &str,
        dependents: Vec<CapabilityProviderRef>,
    ) {
        self.view.send_modify(|view| {
            if let Some(task) = view.tasks.iter_mut().find(|task| task.id == id) {
                task.dependents = dependents;
            }
        });
    }

    /// 连接状态变了：跟着改“在等连接”。`waiting` 是本地的上一份状态，只在变化时写一次。
    pub(super) fn refresh_waiting(
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
    pub(super) fn set_waiting(&self, id: &str, waiting: bool, generation: u64) {
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
    pub(super) fn set_added_tools(&self, id: &str, response: &[u8]) {
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
}
