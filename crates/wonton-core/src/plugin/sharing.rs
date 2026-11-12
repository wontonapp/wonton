use std::sync::Arc;

use wonton_protocol::{PluginState, SharedPlugin, TabDisplay, TabState};

use super::languages::Languages;
use super::{PluginId, PluginManager};
use crate::friends::Friends;
use crate::shares::{PluginOffer, Shares};

#[derive(Debug, thiserror::Error)]
pub enum FriendAdmissionError {
    #[error("friend admission requires the sharing registries and Matrix")]
    NotConfigured,
    #[error(transparent)]
    Friends(#[from] crate::friends::FriendsError),
    #[error(transparent)]
    Storage(#[from] super::store::StoreError),
    #[error(transparent)]
    Matrix(#[from] crate::matrix::MatrixError),
}

impl PluginManager {
    pub fn attach_sharing(&self, friends: Arc<Friends>, shares: Arc<Shares>) {
        assert!(
            self.sharing.set((friends, shares)).is_ok(),
            "sharing registries attach once before serving"
        );
    }

    /// Serialize friend, member and Bot username reservation through durable registration.
    pub async fn lock_person_names(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.bot_allocation.lock().await
    }

    pub(crate) fn attach_person_members(&self, members: Arc<crate::members::Members>) {
        let attached = self.person_members.get_or_init(|| members.clone());
        assert!(
            Arc::ptr_eq(attached, &members),
            "manager shares its member namespace with the gateway"
        );
    }

    /// Call only after observing accepted Matrix friendship, before publishing that friend.
    pub async fn accept_friend(
        &self,
        owner: &crate::owner::OwnerStore,
        members: &crate::members::Members,
        user_id: &str,
        display_name: Option<&str>,
        accepted_at_ms: u64,
    ) -> Result<crate::friends::FriendRecord, FriendAdmissionError> {
        let _allocating = self.lock_person_names().await;
        let (friends, _) = self
            .sharing
            .get()
            .ok_or(FriendAdmissionError::NotConfigured)?;
        let matrix = self
            .matrix
            .as_ref()
            .ok_or(FriendAdmissionError::NotConfigured)?;
        let mut taken = self.bot_localparts()?;
        taken.insert(owner.username().to_owned());
        taken.insert(crate::matrix::ADMIN_LOCALPART.to_owned());
        taken.extend(members.list().into_iter().map(|member| member.username));
        let candidate = friends.candidate_username(user_id)?;
        if taken.contains(&candidate) || matrix.user_exists(&matrix.user_id(&candidate)).await? {
            return Err(crate::friends::FriendsError::UsernameTaken(candidate).into());
        }
        Ok(friends.accept_candidate(
            user_id,
            display_name,
            accepted_at_ms,
            |name| taken.contains(name),
            || Ok(candidate),
        )?)
    }

    pub(crate) fn shared_oidc_client(&self, client: &str) -> bool {
        let Ok(id) = PluginId::parse(client) else {
            return false;
        };
        self.plugins
            .read()
            .expect("plugin registry lock poisoned")
            .get(&id)
            .filter(|entry| entry.status != super::PluginStatus::Uninstalling)
            .and_then(|entry| entry.listed.as_ref())
            .and_then(|listed| listed.manifest.webui.as_ref())
            .is_some_and(|webui| webui.sso == super::manifest::Sso::Oidc && webui.oidc.is_some())
    }

    pub(crate) fn friend_username_taken(&self, username: &str) -> bool {
        self.sharing
            .get()
            .is_some_and(|(friends, _)| friends.taken(username))
    }

    pub(crate) fn reserve_friend_names(&self, taken: &mut std::collections::BTreeSet<String>) {
        if let Some((friends, _)) = self.sharing.get() {
            taken.extend(
                friends
                    .list_including_removed()
                    .into_iter()
                    .map(|friend| friend.username),
            );
        }
    }

    pub(crate) fn revoke_plugin_shares(&self, id: &PluginId) -> Result<(), super::InstallError> {
        if let Some((_, shares)) = self.sharing.get() {
            shares
                .revoke_plugin(id.as_str())
                .map_err(|error| super::InstallError::Storage(error.to_string()))?;
        }
        Ok(())
    }

    pub fn share_offer(&self, id: &PluginId) -> PluginOffer {
        self.with_share_offer(id, |offer| offer)
    }

    /// Keep the registry read lock through persistence so uninstall revokes every prior grant.
    pub(crate) fn with_share_offer<T>(
        &self,
        id: &PluginId,
        commit: impl FnOnce(PluginOffer) -> T,
    ) -> T {
        let plugins = self.plugins.read().expect("plugin registry lock poisoned");
        let entry = plugins.get(id);
        let webui = entry
            .and_then(|entry| entry.listed.as_ref())
            .and_then(|listed| listed.manifest.webui.as_ref());
        commit(PluginOffer {
            plugin_id: id.to_string(),
            installed: entry.is_some_and(|entry| entry.status != super::PluginStatus::Uninstalling),
            has_webui: webui.is_some(),
            per_user_accounts: webui
                .is_some_and(|webui| webui.accounts == Some(super::manifest::Accounts::PerUser)),
        })
    }

    pub fn shared_plugin(&self, id: &PluginId, langs: &Languages) -> Option<SharedPlugin> {
        let plugin = self.installed_plugin(id, langs)?;
        let display = {
            let plugins = self.plugins.read().expect("plugin registry lock poisoned");
            let entry = plugins.get(id)?;
            entry
                .listed
                .as_ref()?
                .manifest
                .webui
                .as_ref()?
                .display
                .clone()
        };
        let state = match plugin.status {
            PluginState::Installing => TabState::Starting,
            PluginState::Running { .. } => TabState::Ready,
            PluginState::Stopped => TabState::Stopped,
            PluginState::Upgrading => TabState::Upgrading,
            PluginState::Uninstalling => return None,
            PluginState::Degraded { reason } => {
                if self
                    .route(&id.slug(), "/")
                    .is_some_and(|route| route.web_serving)
                {
                    TabState::Ready
                } else {
                    TabState::Failed { reason }
                }
            }
            PluginState::Unknown => TabState::Unknown,
        };
        Some(SharedPlugin {
            plugin_id: plugin.id,
            title: plugin.title,
            web_url: plugin.web_url?,
            display: if display == "fullscreen" {
                TabDisplay::Fullscreen
            } else {
                TabDisplay::Standard
            },
            state,
        })
    }
}
