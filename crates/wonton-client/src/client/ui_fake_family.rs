//! 假数据下的成员页：主人视角的一份家人、请求、模板角色，操作只改这份内存里的 ViewModel。

use wonton_protocol::{
    FamilyAccess, FamilyAction, FamilyViewModel, JoinRequestRow, JoinRequestsViewModel, MemberCode,
    MemberInviteView, MemberRow, MemberStatus, MembersViewModel, Permissions, RoleDef, RoleDraft,
    RoleTemplate, RolesViewModel,
};

use std::time::{SystemTime, UNIX_EPOCH};

use super::super::ClientCore;

const FRIEND_ROLE: &str = "friend";

impl ClientCore {
    pub(in crate::client) fn dispatch_ui_fake_family(&self, action: FamilyAction) {
        match action {
            FamilyAction::RefreshFamily | FamilyAction::LeaveFamily => {
                self.views().family.send_replace(demo());
            }
            FamilyAction::InviteMember {
                display_name,
                username,
                role_id,
                ..
            } => self.invite_fake_member(display_name, username, role_id),
            FamilyAction::DismissMemberInvite => {
                self.views().family.send_modify(|view| view.invite = None);
            }
            FamilyAction::ApproveJoinRequest { id } => self.approve_fake_request(&id),
            FamilyAction::RejectJoinRequest { id } => {
                self.views()
                    .family
                    .send_modify(|view| view.join_requests.requests.retain(|r| r.id != id));
            }
            FamilyAction::RemoveMember { username, .. } => {
                self.views()
                    .family
                    .send_modify(|view| view.members.members.retain(|m| m.username != username));
            }
            FamilyAction::CreateRole { draft } => self.create_fake_role(draft),
            FamilyAction::UpdateRole { role_id, draft } => {
                self.views().family.send_modify(|view| {
                    if let Some(existing) = view.roles.roles.iter_mut().find(|r| r.id == role_id) {
                        existing.name = draft.name;
                        existing.permissions = draft.permissions;
                    }
                });
            }
            FamilyAction::DeleteRole { role_id } => {
                self.views().family.send_modify(|view| {
                    view.roles.roles.retain(|r| r.id != role_id);
                });
            }
            FamilyAction::SetDefaultRole { role_id } => {
                self.views().family.send_modify(|view| {
                    view.roles.default_invite_role = Some(role_id);
                });
            }
            FamilyAction::AssignMemberRole { username, role_id } => {
                self.views().family.send_modify(|view| {
                    if let Some(member) = view
                        .members
                        .members
                        .iter_mut()
                        .find(|m| m.username == username)
                    {
                        member.role_id = role_id;
                    }
                });
            }
            FamilyAction::RefreshJoinRequests | FamilyAction::RefreshRoles => {}
            FamilyAction::ResendMemberCode { .. }
            | FamilyAction::RevokeMemberCode { .. }
            | FamilyAction::AssignFriendRole { .. } => {}
        }
    }

    fn invite_fake_member(
        &self,
        display_name: String,
        username: Option<String>,
        role_id: Option<String>,
    ) {
        let username = username.unwrap_or_else(|| display_name.to_lowercase().replace(' ', ""));
        self.views().family.send_modify(|view| {
            view.members.members.push(MemberRow {
                username: username.clone(),
                user_id: format!("@{username}:demo"),
                display_name: display_name.clone(),
                email: None,
                status: MemberStatus::Invited,
                role_id: role_id.unwrap_or_else(|| FRIEND_ROLE.into()),
                role_name: None,
                role_template: Some(RoleTemplate::Friend),
                private_bot: None,
                code: Some(MemberCode {
                    expires_at_ms: in_two_days(),
                    expired: false,
                }),
            });
            view.invite = Some(MemberInviteView {
                username,
                display_name,
                code: "wonton://pair?demo".into(),
                expires_at_ms: in_two_days(),
            });
        });
    }

    fn approve_fake_request(&self, id: &str) {
        self.views().family.send_modify(|view| {
            if let Some(index) = view.join_requests.requests.iter().position(|r| r.id == id) {
                let request = view.join_requests.requests.remove(index);
                view.members.members.push(MemberRow {
                    username: request.username.clone(),
                    user_id: format!("{}:demo", request.username),
                    display_name: request.display_name,
                    email: None,
                    status: MemberStatus::Joined,
                    role_id: request.role_id,
                    role_name: None,
                    role_template: request.role_template,
                    private_bot: None,
                    code: None,
                });
            }
        });
    }

    fn create_fake_role(&self, draft: RoleDraft) {
        self.views().family.send_modify(|view| {
            let id = format!("role-{}", view.roles.roles.len());
            view.roles.roles.push(role(&id, draft, None));
        });
    }
}

/// 邀请码两天后过期，显示“还剩”时是个正常时长。
fn in_two_days() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
        + 2 * 24 * 3600 * 1000
}

fn role(id: &str, draft: RoleDraft, template: Option<RoleTemplate>) -> RoleDef {
    RoleDef {
        id: id.into(),
        name: draft.name,
        template,
        permissions: draft.permissions,
        people: 0,
    }
}

fn member(username: &str, display_name: &str, role_id: &str, status: MemberStatus) -> MemberRow {
    let template = match role_id {
        "admin" => Some(RoleTemplate::Admin),
        "family" => Some(RoleTemplate::Family),
        "child" => Some(RoleTemplate::Child),
        FRIEND_ROLE => Some(RoleTemplate::Friend),
        _ => None,
    };
    MemberRow {
        username: username.into(),
        user_id: format!("@{username}:demo"),
        display_name: display_name.into(),
        email: None,
        status,
        role_id: role_id.into(),
        role_name: None,
        role_template: template,
        private_bot: None,
        code: None,
    }
}

fn demo() -> FamilyViewModel {
    let admin = Permissions {
        manage_members: true,
        ..Permissions::default()
    };
    FamilyViewModel {
        access: FamilyAccess {
            user_id: "@owner:demo".into(),
            username: "owner".into(),
            display_name: Some("Owner".into()),
            is_owner: true,
            role_id: None,
            role_name: None,
            permissions: None,
        },
        members: MembersViewModel {
            loading: false,
            members: vec![
                member("lin", "Lin", "family", MemberStatus::Joined),
                MemberRow {
                    code: Some(MemberCode {
                        expires_at_ms: in_two_days(),
                        expired: false,
                    }),
                    ..member("grace", "Grace", FRIEND_ROLE, MemberStatus::Invited)
                },
            ],
            problem: None,
        },
        join_requests: JoinRequestsViewModel {
            loading: false,
            requests: vec![JoinRequestRow {
                id: "req-1".into(),
                username: "mika".into(),
                display_name: "Mika".into(),
                role_id: FRIEND_ROLE.into(),
                role_name: None,
                role_template: Some(RoleTemplate::Friend),
                device_name: "Pixel".into(),
                requested_at_ms: 1_700_000_000_000,
            }],
            problem: None,
        },
        roles: RolesViewModel {
            loading: false,
            roles: vec![
                role(
                    "admin",
                    RoleDraft {
                        name: String::new(),
                        permissions: admin,
                    },
                    Some(RoleTemplate::Admin),
                ),
                role(
                    "family",
                    RoleDraft {
                        name: String::new(),
                        permissions: Permissions::default(),
                    },
                    Some(RoleTemplate::Family),
                ),
                role(
                    FRIEND_ROLE,
                    RoleDraft {
                        name: String::new(),
                        permissions: Permissions::default(),
                    },
                    Some(RoleTemplate::Friend),
                ),
            ],
            default_invite_role: Some(FRIEND_ROLE.into()),
            problem: None,
        },
        invite: None,
    }
}
