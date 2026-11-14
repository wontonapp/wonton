use super::ClientCore;
use wonton_protocol::{
    Action, CoreError, FriendsViewModel, HomeStatus, PlacedTab, PluginDetail, PluginsViewModel,
    ShareGrantUpdate, SharedAppsState, TabRef,
};

#[uniffi::export]
impl ClientCore {
    /// Active owner-side friends, including their effective shared plugins.
    pub async fn share_recipients(&self) -> Result<FriendsViewModel, CoreError> {
        let scope = self.scope();
        let plugins = scope.plugins.clone();
        scope
            .run(async move { plugins.share_recipients().await })
            .await
    }

    /// Create or replace a grant, then return server-confirmed plugin details.
    pub async fn save_plugin_share(
        &self,
        id: Option<String>,
        grant: ShareGrantUpdate,
    ) -> Result<PluginDetail, CoreError> {
        let scope = self.scope();
        let plugins = scope.plugins.clone();
        scope
            .run(async move { plugins.save_plugin_share(id, grant).await })
            .await
    }

    /// Revoke a grant, then return server-confirmed plugin details.
    pub async fn revoke_plugin_share(
        &self,
        id: String,
        plugin_id: String,
    ) -> Result<PluginDetail, CoreError> {
        let scope = self.scope();
        let plugins = scope.plugins.clone();
        scope
            .run(async move { plugins.revoke_plugin_share(&id, &plugin_id).await })
            .await
    }
}

impl ClientCore {
    pub(super) fn dispatch_sharing(
        &self,
        scope: &crate::scope::Scope,
        action: Action,
    ) -> Result<(), CoreError> {
        match action {
            Action::RefreshSharedApps { owner_user_id } => scope.shared_apps.refresh(owner_user_id),
            Action::OpenSharedApp {
                owner_user_id,
                plugin_id,
            } => {
                scope.pages.open_page(TabRef::SharedApp {
                    owner_user_id: owner_user_id.clone(),
                    plugin_id: plugin_id.clone(),
                })?;
                scope.shared_apps.probe(owner_user_id, plugin_id);
            }
            Action::CreatePluginShare { grant } => scope.plugins.dispatch_share(None, grant),
            Action::UpdatePluginShare { id, grant } => {
                scope.plugins.dispatch_share(Some(id), grant)
            }
            Action::RevokePluginShare { id } => scope.plugins.dispatch_revoke_share(id),
            Action::TabLoadFailed { page, status } => {
                self.shared_page_failed(scope, page, status)?
            }
            Action::TabLoadSucceeded { page } => self.shared_page_loaded(scope, page)?,
            _ => {
                return Err(CoreError::InvalidInput {
                    reason: "invalid sharing action".into(),
                });
            }
        }
        Ok(())
    }

    /// 分享页加载失败：先按来源重探，再记进标签状态。
    fn shared_page_failed(
        &self,
        scope: &crate::scope::Scope,
        page: PlacedTab,
        status: Option<u16>,
    ) -> Result<(), CoreError> {
        if let PlacedTab::Page { tab } = &page {
            self.probe_shared_page(scope, tab);
        }
        scope.pages.load_failed(page, status)
    }

    /// 分享页加载成功：同样重探来源，失败/成功分开记。
    fn shared_page_loaded(
        &self,
        scope: &crate::scope::Scope,
        page: PlacedTab,
    ) -> Result<(), CoreError> {
        if let PlacedTab::Page { tab } = &page {
            self.probe_shared_page(scope, tab);
        }
        scope.pages.load_succeeded(page)
    }

    fn probe_shared_page(&self, scope: &crate::scope::Scope, tab: &TabRef) {
        if let TabRef::SharedApp {
            owner_user_id,
            plugin_id,
        } = tab
        {
            scope
                .shared_apps
                .probe(owner_user_id.clone(), plugin_id.clone());
        }
    }
}

pub(super) fn tab_connected(tab: &TabRef, home: &HomeStatus, plugins: &PluginsViewModel) -> bool {
    match tab {
        TabRef::SharedApp { owner_user_id, .. } => {
            plugins.shared_apps.as_ref().is_some_and(|shared| {
                shared.sources.iter().any(|source| {
                    source.owner_user_id == *owner_user_id
                        && matches!(source.state, SharedAppsState::Ready { .. })
                })
            })
        }
        _ => matches!(home, HomeStatus::Connected),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wonton_protocol::{
        ProxySettings, ShareFailure, ShareProblem, SharedAppsSource, SharedAppsViewModel,
    };

    #[test]
    fn shared_connection_follows_its_source_independently_of_home_and_other_sources() {
        let alice = TabRef::SharedApp {
            owner_user_id: "@alice:a.test".into(),
            plugin_id: "app.same".into(),
        };
        let bob = TabRef::SharedApp {
            owner_user_id: "@bob:b.test".into(),
            plugin_id: "app.same".into(),
        };
        let mut plugins = PluginsViewModel::default();
        assert!(!tab_connected(&alice, &HomeStatus::Connected, &plugins));
        plugins.shared_apps = Some(SharedAppsViewModel {
            sources: vec![SharedAppsSource {
                owner_user_id: "@alice:a.test".into(),
                owner_display_name: None,
                server_name: "a.test".into(),
                state: SharedAppsState::Ready {
                    proxy: ProxySettings {
                        host: "127.0.0.1".into(),
                        port: 1,
                        username: "test".into(),
                        password: "test".into(),
                        domain: "a.test".into(),
                    },
                },
                apps: Vec::new(),
            }],
        });
        for home in [
            HomeStatus::NotConfigured,
            HomeStatus::Connecting,
            HomeStatus::Connected,
        ] {
            assert!(tab_connected(&alice, &home, &plugins));
            assert!(!tab_connected(&bob, &home, &plugins));
        }
        for state in [
            SharedAppsState::Loading,
            SharedAppsState::Failed {
                problem: ShareProblem {
                    reason: ShareFailure::Unreachable,
                    http_status: None,
                    identity: None,
                },
            },
        ] {
            plugins.shared_apps.as_mut().unwrap().sources[0].state = state;
            assert!(!tab_connected(&alice, &HomeStatus::Connected, &plugins));
        }
    }

    #[test]
    fn local_page_connection_continues_to_follow_home() {
        let tab = TabRef::Plugin {
            plugin_id: "app.same".into(),
            page: None,
        };
        let plugins = PluginsViewModel::default();
        for tab in [tab, TabRef::Apps] {
            assert!(tab_connected(&tab, &HomeStatus::Connected, &plugins));
            assert!(!tab_connected(&tab, &HomeStatus::Connecting, &plugins));
            assert!(!tab_connected(&tab, &HomeStatus::NotConfigured, &plugins));
        }
    }
}
