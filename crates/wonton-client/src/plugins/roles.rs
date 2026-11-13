//! 角色页（ADR-0088）：主人建、改、删角色，给成员和好友分配，设默认邀请角色。只有主人能用，
//! 家人设备的每个请求由核心回 `permission_required`。

use std::sync::Arc;

use wonton_l10n::l10n;
use wonton_protocol::{
    Action, CoreError, PluginProblem, RoleAssignment, RoleDef, RoleDraft, RoleList,
};

use super::PluginsLink;
use super::api::READ_TIMEOUT;
use crate::l10n::{CATALOG, language};

const ROLES: [&str; 1] = ["roles"];

/// 一次改动要发的请求。
enum Request {
    Create(RoleDraft),
    Update(String, RoleDraft),
    Delete(String),
    Default(String),
    Member(String, String),
    Friend(String, String),
}

impl PluginsLink {
    /// 角色页的操作。改动成功以后重新取一遍列表：人数和默认角色都由核心算，不在客户端拼。
    pub(crate) fn dispatch_roles(self: &Arc<Self>, action: Action) -> Result<(), CoreError> {
        let request = match action {
            Action::RefreshRoles => {
                self.refresh_roles();
                return Ok(());
            }
            Action::CreateRole { draft } => Request::Create(draft),
            Action::UpdateRole { role_id, draft } => Request::Update(role_id, draft),
            Action::DeleteRole { role_id } => Request::Delete(role_id),
            Action::SetDefaultRole { role_id } => Request::Default(role_id),
            Action::AssignMemberRole { username, role_id } => Request::Member(username, role_id),
            Action::AssignFriendRole { user_id, role_id } => Request::Friend(user_id, role_id),
            _ => {
                return Err(CoreError::InvalidInput {
                    reason: l10n!(CATALOG, language(), "plugins-roles-invalid-action"),
                });
            }
        };
        let link = Arc::clone(self);
        self.runtime.spawn(async move {
            link.home.retry_if_rejected().await;
            let done = match request {
                Request::Create(draft) => link.create_role(&draft).await.map(drop),
                Request::Update(id, draft) => link.update_role(&id, &draft).await.map(drop),
                Request::Delete(id) => link.delete_role(&id).await,
                Request::Default(id) => link.set_default_role(&id).await.map(drop),
                Request::Member(username, id) => {
                    link.assign_member_role(&username, &id).await.map(drop)
                }
                Request::Friend(user_id, id) => {
                    link.assign_friend_role(&user_id, &id).await.map(drop)
                }
            };
            let ok = done.is_ok();
            link.view.send_modify(|view| match done {
                Ok(()) => view.roles.problem = None,
                Err(problem) => view.roles.problem = Some(problem),
            });
            if ok {
                link.spawn_roles();
            }
        });
        Ok(())
    }

    /// 打开角色页：取一遍列表，之前的错误清掉。
    pub(crate) fn refresh_roles(self: &Arc<Self>) {
        if self.external || self.connecting() {
            return;
        }
        let link = Arc::clone(self);
        self.runtime.spawn(async move {
            link.home.retry_if_rejected().await;
            link.spawn_roles();
        });
    }

    pub(crate) fn spawn_roles(self: &Arc<Self>) {
        let link = Arc::clone(self);
        self.view.send_modify(|view| view.roles.loading = true);
        self.runtime.spawn(async move {
            let fetched = link.roles().await;
            link.view.send_modify(|view| {
                view.roles.loading = false;
                match fetched {
                    Ok(list) => {
                        view.roles.roles = list.roles;
                        view.roles.default_invite_role = Some(list.default_invite_role);
                        view.roles.problem = None;
                    }
                    Err(problem) => view.roles.problem = Some(problem),
                }
            });
        });
    }

    async fn roles(&self) -> Result<RoleList, PluginProblem> {
        self.api()?.get(&ROLES, &[], READ_TIMEOUT).await
    }

    async fn create_role(&self, draft: &RoleDraft) -> Result<RoleDef, PluginProblem> {
        self.api()?.post(&ROLES, draft, READ_TIMEOUT).await
    }

    async fn update_role(
        &self,
        role_id: &str,
        draft: &RoleDraft,
    ) -> Result<RoleDef, PluginProblem> {
        self.api()?
            .put(&["roles", role_id], draft, READ_TIMEOUT)
            .await
    }

    async fn delete_role(&self, role_id: &str) -> Result<(), PluginProblem> {
        self.api()?
            .delete(&["roles", role_id], &[], READ_TIMEOUT)
            .await
    }

    async fn set_default_role(&self, role_id: &str) -> Result<RoleList, PluginProblem> {
        let assignment = RoleAssignment {
            role_id: role_id.to_owned(),
        };
        self.api()?
            .put(&["roles", "default-invite"], &assignment, READ_TIMEOUT)
            .await
    }

    async fn assign_member_role(
        &self,
        username: &str,
        role_id: &str,
    ) -> Result<RoleList, PluginProblem> {
        let assignment = RoleAssignment {
            role_id: role_id.to_owned(),
        };
        self.api()?
            .put(&["members", username, "role"], &assignment, READ_TIMEOUT)
            .await
    }

    async fn assign_friend_role(
        &self,
        user_id: &str,
        role_id: &str,
    ) -> Result<RoleList, PluginProblem> {
        let assignment = RoleAssignment {
            role_id: role_id.to_owned(),
        };
        self.api()?
            .put(&["friends", user_id, "role"], &assignment, READ_TIMEOUT)
            .await
    }
}
