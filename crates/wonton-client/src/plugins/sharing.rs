use super::{PluginsLink, READ_TIMEOUT, api::to_core_error};
use wonton_protocol::{CoreError, FriendsViewModel, PluginDetail, ShareGrant, ShareGrantUpdate};

impl PluginsLink {
    pub(crate) async fn share_recipients(&self) -> Result<FriendsViewModel, CoreError> {
        self.api()
            .map_err(to_core_error)?
            .get(&["friends"], &[], READ_TIMEOUT)
            .await
            .map_err(to_core_error)
    }

    pub(crate) async fn save_plugin_share(
        &self,
        id: Option<String>,
        grant: ShareGrantUpdate,
    ) -> Result<PluginDetail, CoreError> {
        let api = self.api().map_err(to_core_error)?;
        let _: ShareGrant = match id {
            Some(id) => api.put(&["shares", &id], &grant, READ_TIMEOUT).await,
            None => api.post(&["shares"], &grant, READ_TIMEOUT).await,
        }
        .map_err(to_core_error)?;
        api.get(&["plugins", &grant.plugin_id], &[], READ_TIMEOUT)
            .await
            .map_err(to_core_error)
    }

    pub(crate) async fn revoke_plugin_share(
        &self,
        id: &str,
        plugin: &str,
    ) -> Result<PluginDetail, CoreError> {
        let api = self.api().map_err(to_core_error)?;
        api.delete(&["shares", id], &[], READ_TIMEOUT)
            .await
            .map_err(to_core_error)?;
        api.get(&["plugins", plugin], &[], READ_TIMEOUT)
            .await
            .map_err(to_core_error)
    }

    pub(crate) fn dispatch_share(
        self: &std::sync::Arc<Self>,
        id: Option<String>,
        grant: ShareGrantUpdate,
    ) {
        let link = self.clone();
        self.runtime.spawn(async move {
            let result = link.save_plugin_share(id, grant).await;
            link.record_share_result(result.map(drop));
        });
    }

    pub(crate) fn dispatch_revoke_share(self: &std::sync::Arc<Self>, id: String) {
        let link = self.clone();
        self.runtime.spawn(async move {
            let result = async {
                link.api()
                    .map_err(to_core_error)?
                    .delete(&["shares", &id], &[], READ_TIMEOUT)
                    .await
                    .map_err(to_core_error)
            }
            .await;
            link.record_share_result(result);
        });
    }

    fn record_share_result(&self, result: Result<(), CoreError>) {
        if let Err(error) = result {
            self.view.send_modify(|view| {
                view.problem = Some(wonton_protocol::PluginProblem {
                    reason: wonton_protocol::PluginFailure::Upstream,
                    min_version: None,
                    settings: None,
                    message: error.to_string(),
                })
            });
        }
    }
}
