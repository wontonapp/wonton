//! 假核心里的成员、角色和加入请求：和真核心一样按状态记账，测试用驱动它发请求、断言它收到了什么。
//!
//! 只有这一块的状态在这里；隧道和请求记录用 `plugins::rig` 的。

use std::collections::HashMap;
use std::sync::Mutex;

use axum::http::{Method, StatusCode};
use axum::response::{IntoResponse, Response};
use wonton_protocol::{
    JoinRequest, JoinRequestList, Member, MemberInvite, MemberList, MemberPairingCode,
    MemberRemoval, MemberStatus, Permissions, RoleAssignment, RoleDef, RoleDraft, RoleList,
    RoleTemplate,
};

#[derive(Default)]
pub struct FamilyFake {
    me: Mutex<Option<serde_json::Value>>,
    members: Mutex<Vec<Member>>,
    roles: Mutex<Vec<RoleDef>>,
    default_invite_role: Mutex<String>,
    requests: Mutex<Vec<JoinRequest>>,
    codes: Mutex<HashMap<String, String>>,
    issued: Mutex<u64>,
    /// 下一个请求注定失败的接口和回应（状态、类别、响应体）。
    failures: Mutex<HashMap<(Method, String), (u16, Option<String>, String)>>,
}

fn member(username: &str, role_id: &str, status: MemberStatus) -> Member {
    Member {
        username: username.to_owned(),
        user_id: format!("@{username}:wonton.localhost"),
        display_name: username.to_owned(),
        email: None,
        status,
        role_id: role_id.to_owned(),
        private_bot: None,
        pairing_code_expires_at_ms: None,
    }
}

fn role(id: &str, name: &str, template: Option<RoleTemplate>, permissions: Permissions) -> RoleDef {
    RoleDef {
        id: id.to_owned(),
        name: name.to_owned(),
        template,
        permissions,
        people: 0,
    }
}

impl FamilyFake {
    pub fn owner(&self) {
        self.set_me(
            serde_json::json!({
                "user_id": "@owner:wonton.localhost",
                "username": "owner",
                "is_owner": true,
                "display_name": "主人",
            }),
            Some("admin"),
        );
    }

    /// 一个没有“管理成员”的家人。
    pub fn member_without_managing(&self) {
        self.set_me(
            serde_json::json!({
                "user_id": "@bob:wonton.localhost",
                "username": "bob",
                "is_owner": false,
                "role_id": "friend",
                "role_name": "朋友",
                "display_name": "Bob",
                "permissions": Permissions::strictest(),
            }),
            None,
        );
    }

    fn set_me(&self, me: serde_json::Value, invite_role: Option<&str>) {
        *self.me.lock().unwrap() = Some(me);
        *self.roles.lock().unwrap() = vec![
            role("admin", "", Some(RoleTemplate::Admin), Permissions::owner()),
            role(
                "child",
                "孩子",
                Some(RoleTemplate::Child),
                Permissions {
                    private_bot: true,
                    ..Permissions::strictest()
                },
            ),
            role(
                "friend",
                "朋友",
                Some(RoleTemplate::Friend),
                Permissions::strictest(),
            ),
        ];
        if let Some(invite_role) = invite_role {
            *self.default_invite_role.lock().unwrap() = invite_role.to_owned();
        }
    }

    pub fn add_member(&self, username: &str, role_id: &str) {
        self.members
            .lock()
            .unwrap()
            .push(member(username, role_id, MemberStatus::Joined));
    }

    /// 假装被邀请的人在另一台设备上交了码。
    pub fn join_request(&self, id: &str, username: &str, role_id: &str) {
        self.requests.lock().unwrap().push(JoinRequest {
            id: id.to_owned(),
            username: username.to_owned(),
            display_name: username.to_owned(),
            role_id: role_id.to_owned(),
            device_name: "小米".to_owned(),
            requested_at_ms: 1,
        });
    }

    /// 把码的期限改到过去。
    pub fn expire_code(&self, username: &str) {
        if let Some(member) = self
            .members
            .lock()
            .unwrap()
            .iter_mut()
            .find(|member| member.username == username)
        {
            member.pairing_code_expires_at_ms = Some(1);
        }
    }

    pub fn member(&self, username: &str) -> Option<Member> {
        self.members
            .lock()
            .unwrap()
            .iter()
            .find(|member| member.username == username)
            .cloned()
    }

    pub fn code(&self, username: &str) -> Option<String> {
        self.codes.lock().unwrap().get(username).cloned()
    }

    /// 下一次这个接口失败。
    pub fn fail(&self, method: Method, path: &str, status: u16, kind: Option<&str>, body: &str) {
        self.failures.lock().unwrap().insert(
            (method, path.to_owned()),
            (status, kind.map(str::to_owned), body.to_owned()),
        );
    }
}

fn json(value: impl serde::Serialize) -> Response {
    (StatusCode::OK, serde_json::to_string(&value).unwrap()).into_response()
}

fn error(status: u16, kind: &str, body: &str) -> Response {
    let mut response = (StatusCode::from_u16(status).unwrap(), body.to_owned()).into_response();
    response
        .headers_mut()
        .insert("x-wonton-error", kind.parse().unwrap());
    response
}

fn invited(member: &Member) -> bool {
    member.status == MemberStatus::Invited
}

/// 路径里的 `%XX` 还原成字节，用户名可能是中文。
fn decode(path: &str) -> String {
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).ok();
            if let Some(byte) = hex.and_then(|hex| u8::from_str_radix(hex, 16).ok()) {
                out.push(byte);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}
/// 这个请求归成员接口时回一份回应；不归时 `None`。
pub fn respond(fake: &FamilyFake, method: &Method, path: &str, body: &[u8]) -> Option<Response> {
    if let Some((status, kind, body)) = fake
        .failures
        .lock()
        .unwrap()
        .remove(&(method.clone(), path.to_owned()))
    {
        return Some(match kind {
            Some(kind) => error(status, &kind, &body),
            None => (StatusCode::from_u16(status).unwrap(), body).into_response(),
        });
    }
    let (owner, managing) = match fake.me.lock().unwrap().as_ref() {
        Some(me) => (
            me["is_owner"].as_bool() == Some(true),
            me["is_owner"].as_bool() == Some(true)
                || me["permissions"]["manage_members"].as_bool() == Some(true),
        ),
        None => (true, true),
    };
    let members_path = path.starts_with("/_wonton/api/members");
    let roles_path =
        path.starts_with("/_wonton/api/roles") || path.starts_with("/_wonton/api/friends/");
    let denied = |kind: &str| {
        Some(error(
            403,
            "permission_required",
            &format!("{{\"missing\":\"{kind}\"}}"),
        ))
    };
    if members_path && !managing && !path.ends_with("/me") {
        return denied("manage_members");
    }
    if roles_path && !owner {
        return denied("owner");
    }
    let decoded = decode(path);
    let segments: Vec<&str> = decoded
        .strip_prefix("/_wonton/api/")
        .unwrap_or("")
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect();
    match (method, segments.as_slice()) {
        (&Method::GET, ["me"]) => fake.me.lock().unwrap().as_ref().map(json),
        (&Method::GET, ["members"]) => Some(json(MemberList {
            members: fake.members.lock().unwrap().clone(),
        })),
        (&Method::POST, ["members"]) => {
            let invite: MemberInvite = serde_json::from_slice(body).ok()?;
            let role_id = invite
                .role_id
                .clone()
                .unwrap_or_else(|| fake.default_invite_role.lock().unwrap().clone());
            let username = invite
                .username
                .clone()
                .unwrap_or_else(|| invite.display_name.clone());
            let created = Member {
                display_name: invite.display_name,
                status: MemberStatus::Invited,
                ..member(&username, &role_id, MemberStatus::Invited)
            };
            fake.members.lock().unwrap().push(created.clone());
            Some(
                (
                    StatusCode::CREATED,
                    serde_json::to_string(&created).unwrap(),
                )
                    .into_response(),
            )
        }
        (&Method::POST, ["members", username, "pairing-code"]) => {
            let mut members = fake.members.lock().unwrap();
            let found = members
                .iter_mut()
                .find(|member| member.username == *username && invited(member))?;
            let expires_at_ms = u64::MAX;
            found.pairing_code_expires_at_ms = Some(expires_at_ms);
            let mut issued = fake.issued.lock().unwrap();
            *issued += 1;
            let code = format!("wonton://pair/{username}?n={issued}");
            fake.codes
                .lock()
                .unwrap()
                .insert((*username).to_owned(), code.clone());
            Some(json(MemberPairingCode {
                code,
                expires_at_ms,
            }))
        }
        (&Method::DELETE, ["members", username, "pairing-code"]) => {
            fake.codes.lock().unwrap().remove(*username);
            fake.members
                .lock()
                .unwrap()
                .iter_mut()
                .find(|member| member.username == *username)
                .map(|member| member.pairing_code_expires_at_ms = None);
            Some(StatusCode::NO_CONTENT.into_response())
        }
        (&Method::POST, ["members", username, "remove"]) => {
            let removal: MemberRemoval = serde_json::from_slice(body).ok()?;
            let _ = removal;
            let mut members = fake.members.lock().unwrap();
            let found = members
                .iter_mut()
                .find(|member| member.username == *username)?;
            found.status = MemberStatus::Removed;
            fake.codes.lock().unwrap().remove(*username);
            Some(json(found.clone()))
        }
        (&Method::PUT, ["members", username, "role"]) => {
            let assignment: RoleAssignment = serde_json::from_slice(body).ok()?;
            {
                let mut members = fake.members.lock().unwrap();
                let found = members
                    .iter_mut()
                    .find(|member| member.username == *username)?;
                found.role_id = assignment.role_id;
            }
            Some(json(list(fake)))
        }
        (&Method::GET, ["members", "join-requests"]) => Some(json(JoinRequestList {
            requests: fake.requests.lock().unwrap().clone(),
        })),
        (&Method::POST, ["members", "join-requests", id, decision]) => {
            let mut requests = fake.requests.lock().unwrap();
            let index = requests.iter().position(|request| request.id == *id)?;
            let request = requests.remove(index);
            if *decision == "approve" {
                let mut members = fake.members.lock().unwrap();
                let found = members
                    .iter_mut()
                    .find(|member| member.username == request.username)?;
                found.status = MemberStatus::Joined;
            } else {
                fake.codes.lock().unwrap().remove(&request.username);
            }
            Some(StatusCode::NO_CONTENT.into_response())
        }
        (&Method::GET, ["roles"]) => Some(json(list(fake))),
        (&Method::POST, ["roles"]) => {
            let draft: RoleDraft = serde_json::from_slice(body).ok()?;
            let created = role(
                &format!("role-{}", fake.roles.lock().unwrap().len()),
                &draft.name,
                None,
                draft.permissions,
            );
            fake.roles.lock().unwrap().push(created.clone());
            Some(
                (
                    StatusCode::CREATED,
                    serde_json::to_string(&created).unwrap(),
                )
                    .into_response(),
            )
        }
        (&Method::PUT, ["roles", "default-invite"]) => {
            let assignment: RoleAssignment = serde_json::from_slice(body).ok()?;
            *fake.default_invite_role.lock().unwrap() = assignment.role_id;
            Some(json(list(fake)))
        }
        (&Method::PUT, ["roles", id]) => {
            let draft: RoleDraft = serde_json::from_slice(body).ok()?;
            let mut roles = fake.roles.lock().unwrap();
            let found = roles.iter_mut().find(|role| role.id == *id)?;
            found.name = draft.name;
            found.permissions = draft.permissions;
            Some(json(found.clone()))
        }
        (&Method::DELETE, ["roles", id]) => {
            fake.roles.lock().unwrap().retain(|role| role.id != *id);
            Some(StatusCode::NO_CONTENT.into_response())
        }
        _ => None,
    }
}

fn list(fake: &FamilyFake) -> RoleList {
    RoleList {
        roles: fake.roles.lock().unwrap().clone(),
        default_invite_role: fake.default_invite_role.lock().unwrap().clone(),
    }
}
