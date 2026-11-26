//! 角色页（ADR-0088）：主人建、改、删角色，给成员和好友分配，设默认邀请角色。只有主人能用，
//! 家人设备的每个请求由核心回 `permission_required`。

use std::sync::Arc;

use reqwest::Method;
use wonton_protocol::{FamilyAction, FamilyProblem, RoleAssignment, RoleDef, RoleDraft, RoleList};

use super::FamilyLink;
use crate::plugins::api::READ_TIMEOUT;

/// 一次改动要发的请求。
pub(super) enum Request {
    Create(RoleDraft),
    Update(String, RoleDraft),
    Delete(String),
    Default(String),
    Member(String, String),
    Friend(String, String),
}

impl FamilyLink {
    /// 角色操作认领：不是角色操作的返回 `None`。
    pub(super) fn roles_action(&self, action: FamilyAction) -> Option<Request> {
        Some(match action {
            FamilyAction::CreateRole { draft } => Request::Create(draft),
            FamilyAction::UpdateRole { role_id, draft } => Request::Update(role_id, draft),
            FamilyAction::DeleteRole { role_id } => Request::Delete(role_id),
            FamilyAction::SetDefaultRole { role_id } => Request::Default(role_id),
            FamilyAction::AssignMemberRole { username, role_id } => {
                Request::Member(username, role_id)
            }
            FamilyAction::AssignFriendRole { user_id, role_id } => {
                Request::Friend(user_id, role_id)
            }
            _ => return None,
        })
    }

    /// 发角色改动。成功后重取角色和成员：人数、默认角色和成员那一行的角色都由核心算。
    pub(super) fn mutate(self: &Arc<Self>, request: Request) {
        let link = Arc::clone(self);
        self.spawn(async move {
            let done = async {
                let api = link.api()?;
                match request {
                    Request::Create(draft) => api
                        .call::<RoleDef>(Method::POST, &["roles"], Some(&draft), READ_TIMEOUT)
                        .await
                        .map(drop),
                    Request::Update(id, draft) => api
                        .call::<RoleDef>(Method::PUT, &["roles", &id], Some(&draft), READ_TIMEOUT)
                        .await
                        .map(drop),
                    Request::Delete(id) => api
                        .call_empty(Method::DELETE, &["roles", &id], None::<&()>, READ_TIMEOUT)
                        .await
                        .map(drop),
                    Request::Default(id) => {
                        let assignment = RoleAssignment { role_id: id };
                        api.call::<RoleList>(
                            Method::PUT,
                            &["roles", "default-invite"],
                            Some(&assignment),
                            READ_TIMEOUT,
                        )
                        .await
                        .map(drop)
                    }
                    Request::Member(username, id) => {
                        let assignment = RoleAssignment { role_id: id };
                        api.call::<RoleList>(
                            Method::PUT,
                            &["members", &username, "role"],
                            Some(&assignment),
                            READ_TIMEOUT,
                        )
                        .await
                        .map(drop)
                    }
                    Request::Friend(user_id, id) => {
                        let assignment = RoleAssignment { role_id: id };
                        let path = ["friends", &user_id, "role"].map(str::to_owned);
                        let path: Vec<&str> = path.iter().map(String::as_str).collect();
                        api.call::<RoleList>(Method::PUT, &path, Some(&assignment), READ_TIMEOUT)
                            .await
                            .map(drop)
                    }
                }
            }
            .await;
            let ok = done.is_ok();
            link.view.send_modify(|view| match done {
                Ok(()) => view.roles.problem = None,
                Err(problem) => view.roles.problem = Some(problem),
            });
            if ok {
                link.refresh(false);
            }
        });
    }

    /// 打开角色页：取一遍列表，之前的错误清掉。
    pub(super) fn refresh_roles(self: &Arc<Self>) {
        if !self.available() || !self.view.borrow().access.manages_roles() {
            return;
        }
        let link = Arc::clone(self);
        self.spawn(async move {
            link.view.send_modify(|view| view.roles.loading = true);
            let fetched = link.fetch_roles().await;
            link.view.send_modify(|view| {
                view.roles.loading = false;
                apply(view, fetched);
            });
        });
    }

    pub(super) async fn fetch_roles(&self) -> Result<RoleList, FamilyProblem> {
        let api = self.api()?;
        api.call(Method::GET, &["roles"], None::<&()>, READ_TIMEOUT)
            .await
    }
}

fn apply(view: &mut wonton_protocol::FamilyViewModel, fetched: Result<RoleList, FamilyProblem>) {
    match fetched {
        Ok(list) => {
            view.roles.roles = list.roles;
            view.roles.default_invite_role = Some(list.default_invite_role);
            view.roles.problem = None;
        }
        Err(problem) => view.roles.problem = Some(problem),
    }
}
