use super::{PluginsLink, READ_TIMEOUT, api::to_core_error};
use wonton_protocol::{
    CoreError, FriendsList, FriendsState, FriendsViewModel, PluginDetail, ShareGrant,
    ShareGrantUpdate,
};

impl PluginsLink {
    /// 只取核心存着的好友和分组：分享对象只能是核心认得的人，请求和屏蔽不用从 Matrix 拼。
    pub(crate) async fn share_recipients(&self) -> Result<FriendsViewModel, CoreError> {
        let list: FriendsList = self
            .api()
            .map_err(to_core_error)?
            .get(&["friends"], &[], READ_TIMEOUT)
            .await
            .map_err(to_core_error)?;
        Ok(recipients(list))
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

/// 好友页的请求、屏蔽和自己的链接不在核心名单里，这里留空；壳只读 `friends` 和 `groups`。
pub(crate) fn recipients(list: FriendsList) -> FriendsViewModel {
    FriendsViewModel {
        state: FriendsState::Ready,
        friends: list.friends,
        groups: list.groups,
        incoming: Vec::new(),
        outgoing: Vec::new(),
        blocked: Vec::new(),
        can_block_federation: list.can_block_federation,
        my_link: None,
    }
}
