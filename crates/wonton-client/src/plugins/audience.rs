//! 按角色开放应用；选择和提示的规则由客户端核心统一给两端壳。

use std::sync::Arc;

use wonton_protocol::{CoreError, PluginAudience, PluginProblem};

use super::PluginsLink;
use super::api::{READ_TIMEOUT, to_core_error};

impl PluginsLink {
    pub(crate) async fn plugin_audience(&self, id: &str) -> Result<PluginAudience, CoreError> {
        self.api()
            .map_err(to_core_error)?
            .get(&["plugins", id, "audience"], &[], READ_TIMEOUT)
            .await
            .map_err(to_core_error)
    }

    pub(crate) async fn put_plugin_audience(
        &self,
        id: &str,
        audience: &PluginAudience,
    ) -> Result<PluginAudience, PluginProblem> {
        self.api()?
            .put(&["plugins", id, "audience"], audience, READ_TIMEOUT)
            .await
    }

    pub(crate) fn set_plugin_audience(
        self: &Arc<Self>,
        id: String,
        audience: PluginAudience,
    ) -> Result<(), CoreError> {
        let api = self.api().map_err(to_core_error)?;
        let link = Arc::clone(self);
        let generation = self.generation.load(std::sync::atomic::Ordering::SeqCst);
        self.runtime.spawn(async move {
            let result: Result<PluginAudience, PluginProblem> = api
                .put(&["plugins", &id, "audience"], &audience, READ_TIMEOUT)
                .await;
            if !link.is_current(generation) {
                return;
            }
            link.record(super::Listing::Installed, result.map(drop));
            link.refresh_installed();
        });
        Ok(())
    }
}
