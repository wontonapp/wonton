//! 启动只采用服务器的结果；不伪造任务类型或运行状态。失败保留在页面上，包括连接中断。

use std::sync::Arc;
use std::sync::atomic::Ordering;

use wonton_l10n::l10n;
use wonton_protocol::{
    CoreError, InstalledPlugin, PluginProblem, PluginStart, PluginStartState, PluginState,
    PluginsViewModel,
};

use super::api::INSTALL_TIMEOUT;
use super::{Listing, PluginsLink, lock};
use crate::l10n::{CATALOG, language};

impl PluginsLink {
    pub(crate) fn start(self: &Arc<Self>, id: String) -> Result<(), CoreError> {
        let generation = self.generation.load(Ordering::SeqCst);
        let server = self.home.view().server_id;
        {
            let mut starts = lock(&self.starts);
            if starts.get(&id) == Some(&generation)
                || self
                    .view
                    .borrow()
                    .tasks
                    .iter()
                    .any(|task| task.id == id && task.state.in_progress())
            {
                return Err(CoreError::NotReady {
                    reason: l10n!(CATALOG, language(), "plugins-start-busy", id = id),
                });
            }
            starts.insert(id.clone(), generation);
        }
        // 连接和语言在派发时固定，不能等异步任务开始才取到另一台服务器。
        let api = self.api();
        self.record(Listing::Start(id.clone()), Ok(()));
        self.set_start(&id, Some(PluginStartState::Starting));
        let link = Arc::clone(self);
        self.runtime.spawn(async move {
            let result: Result<(), PluginProblem> = async {
                if !link.is_current_home(generation, &server) {
                    return Ok(());
                }
                let api = api?;
                let _: InstalledPlugin = api
                    .post(
                        &["plugins", &id, "start"],
                        &serde_json::json!({}),
                        INSTALL_TIMEOUT,
                    )
                    .await?;
                if !link.is_current_home(generation, &server) {
                    return Ok(());
                }
                link.fetch_installed_from(&api, generation, &server).await
            }
            .await;
            let mut starts = lock(&link.starts);
            if starts.get(&id) == Some(&generation) {
                starts.remove(&id);
            }
            if link.is_current_home(generation, &server) {
                let state = result
                    .as_ref()
                    .err()
                    .map(|problem| PluginStartState::Failed {
                        problem: problem.clone(),
                    });
                link.set_start(&id, state);
                link.record(Listing::Start(id), result);
            }
        });
        Ok(())
    }

    fn set_start(&self, id: &str, state: Option<PluginStartState>) {
        self.view.send_modify(|view| {
            view.starts.retain(|start| start.plugin_id != id);
            if let Some(state) = state {
                view.starts.push(PluginStart {
                    plugin_id: id.to_owned(),
                    state,
                });
            }
            Self::reconcile_starts(view);
        });
    }

    pub(super) fn reconcile_starts(view: &mut PluginsViewModel) -> bool {
        let Some(installed) = &view.installed else {
            return false;
        };
        let before = view.starts.len();
        view.starts.retain(|start| {
            !installed.plugins.iter().any(|plugin| {
                plugin.id == start.plugin_id && matches!(plugin.status, PluginState::Running { .. })
            })
        });
        before != view.starts.len()
    }
}
