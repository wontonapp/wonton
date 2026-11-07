//! 启动只采用服务器的结果；不伪造任务类型或运行状态。失败保留在页面上，包括连接中断。

use std::sync::Arc;
use std::sync::atomic::Ordering;

use wonton_l10n::l10n;
use wonton_protocol::{CoreError, InstalledPlugin, PluginProblem, PluginTaskState};

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
                    .any(|task| task.id == id && task.state == PluginTaskState::Running)
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
                link.record(Listing::Start(id), result);
            }
        });
        Ok(())
    }
}
