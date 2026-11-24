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

/// 一个注定失败的请求的回应：状态、类别、响应体；没类别时按裸状态和响应体回。
type ForcedFailure = (u16, Option<String>, String);

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
    failures: Mutex<HashMap<(Method, String), ForcedFailure>>,
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

    /// 能管成员、但不是主人的管理员。
    pub fn manager(&self) {
        let permissions = Permissions {
            manage_members: true,
            ..Permissions::strictest()
        };
        self.set_me(
            serde_json::json!({
                "user_id": "@bob:wonton.localhost",
                "username": "bob",
                "is_owner": false,
                "role_id": "admin",
                "role_name": "管理员",
                "display_name": "Bob",
                "permissions": permissions,
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

    pub fn add_role(&self, id: &str, name: &str) {
        self.roles
            .lock()
            .unwrap()
            .push(role(id, name, None, Permissions::strictest()));
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
    if let Some(reply) = forced(fake, method, path) {
        return Some(reply);
    }
    if let Some(denied) = permission(fake, path) {
        return Some(denied);
    }
    let decoded = decode(path);
    let segments: Vec<&str> = decoded
        .strip_prefix("/_wonton/api/")
        .unwrap_or("")
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect();
    match segments.first().copied() {
        Some("me") if method == Method::GET => fake.me.lock().unwrap().as_ref().map(json),
        Some("members") => member_reply(fake, method, &segments, body),
        Some("roles") => role_reply(fake, method, &segments, body),
        Some("friends") => friend_reply(fake, method, &segments, body),
        _ => None,
    }
}

/// 测试塞进来的失败比真实处理优先。
fn forced(fake: &FamilyFake, method: &Method, path: &str) -> Option<Response> {
    let (status, kind, body) = fake
        .failures
        .lock()
        .unwrap()
        .remove(&(method.clone(), path.to_owned()))?;
    Some(match kind {
        Some(kind) => error(status, &kind, &body),
        None => (StatusCode::from_u16(status).unwrap(), body).into_response(),
    })
}

/// 权限不够时回 403，带上缺的项：成员接口要“管理成员”，角色接口只有主人能进。
fn permission(fake: &FamilyFake, path: &str) -> Option<Response> {
    let me = fake.me.lock().unwrap();
    let (owner, managing) = match me.as_ref() {
        Some(me) => (
            me["is_owner"].as_bool() == Some(true),
            me["is_owner"].as_bool() == Some(true)
                || me["permissions"]["manage_members"].as_bool() == Some(true),
        ),
        None => (true, true),
    };
    let denied = |kind: &str| {
        Some(error(
            403,
            "permission_required",
            &format!("{{\"missing\":\"{kind}\"}}"),
        ))
    };
    if path.starts_with("/_wonton/api/members") && !managing && !path.ends_with("/me") {
        return denied("manage_members");
    }
    if (path.starts_with("/_wonton/api/roles") || path.starts_with("/_wonton/api/friends/"))
        && !owner
    {
        return denied("owner");
    }
    None
}

/// 成员接口：成员列表、邀请、码、移除、改角色和加入请求。
fn member_reply(
    fake: &FamilyFake,
    method: &Method,
    segments: &[&str],
    body: &[u8],
) -> Option<Response> {
    match (method, segments) {
        (&Method::GET, ["members"]) => Some(json(MemberList {
            members: fake.members.lock().unwrap().clone(),
        })),
        (&Method::POST, ["members"]) => Some(invite(fake, body)),
        (&Method::GET, ["members", "join-requests"]) => Some(json(JoinRequestList {
            requests: fake.requests.lock().unwrap().clone(),
        })),
        (&Method::POST, ["members", "join-requests", id, decision]) => decide(fake, id, decision),
        (&Method::POST, ["members", username, "pairing-code"]) => issue(fake, username),
        (&Method::DELETE, ["members", username, "pairing-code"]) => revoke(fake, username),
        (&Method::POST, ["members", username, "remove"]) => remove(fake, username, body),
        (&Method::PUT, ["members", username, "role"]) => assign(fake, username, body),
        _ => None,
    }
}

/// 邀请：没带角色时用默认邀请角色。
fn invite(fake: &FamilyFake, body: &[u8]) -> Response {
    let invite: MemberInvite = serde_json::from_slice(body).unwrap();
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
    (
        StatusCode::CREATED,
        serde_json::to_string(&created).unwrap(),
    )
        .into_response()
}

/// 给被邀请的人一个码。
fn issue(fake: &FamilyFake, username: &str) -> Option<Response> {
    let mut members = fake.members.lock().unwrap();
    let found = members
        .iter_mut()
        .find(|member| member.username == username && invited(member))?;
    let expires_at_ms = u64::MAX;
    found.pairing_code_expires_at_ms = Some(expires_at_ms);
    let mut issued = fake.issued.lock().unwrap();
    *issued += 1;
    let code = format!("wonton://pair/{username}?n={issued}");
    fake.codes
        .lock()
        .unwrap()
        .insert(username.to_owned(), code.clone());
    Some(json(MemberPairingCode {
        code,
        expires_at_ms,
    }))
}

/// 作废这个人的码，成员行上的过期时间也清掉。
fn revoke(fake: &FamilyFake, username: &str) -> Option<Response> {
    fake.codes.lock().unwrap().remove(username);
    if let Some(member) = fake
        .members
        .lock()
        .unwrap()
        .iter_mut()
        .find(|member| member.username == username)
    {
        member.pairing_code_expires_at_ms = None;
    }
    Some(StatusCode::NO_CONTENT.into_response())
}

/// 移除：成员状态改掉、码作废，回新的成员行。
fn remove(fake: &FamilyFake, username: &str, body: &[u8]) -> Option<Response> {
    serde_json::from_slice::<MemberRemoval>(body).ok()?;
    let mut members = fake.members.lock().unwrap();
    let found = members
        .iter_mut()
        .find(|member| member.username == username)?;
    found.status = MemberStatus::Removed;
    fake.codes.lock().unwrap().remove(username);
    Some(json(found.clone()))
}

/// 换角色：改完回整份角色列表。
fn assign(fake: &FamilyFake, username: &str, body: &[u8]) -> Option<Response> {
    let assignment: RoleAssignment = serde_json::from_slice(body).ok()?;
    let mut members = fake.members.lock().unwrap();
    let found = members
        .iter_mut()
        .find(|member| member.username == username)?;
    found.role_id = assignment.role_id;
    Some(json(list(fake)))
}

/// 好友的角色：只认换角色，角色得在名单里，改完回整份角色列表。
fn friend_reply(
    fake: &FamilyFake,
    method: &Method,
    segments: &[&str],
    body: &[u8],
) -> Option<Response> {
    let (&Method::PUT, ["friends", _user_id, "role"]) = (method, segments) else {
        return None;
    };
    let assignment: RoleAssignment = serde_json::from_slice(body).ok()?;
    let known = fake
        .roles
        .lock()
        .unwrap()
        .iter()
        .any(|role| role.id == assignment.role_id);
    known.then(|| json(list(fake)))
}

/// 同意或拒绝一条加入请求：同意把人改成已加入，拒绝连同码作废。
fn decide(fake: &FamilyFake, id: &str, decision: &str) -> Option<Response> {
    let mut requests = fake.requests.lock().unwrap();
    let index = requests.iter().position(|request| request.id == id)?;
    let request = requests.remove(index);
    if decision == "approve" {
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

/// 角色接口：写操作只有主人能进，列表里带默认邀请角色。
fn role_reply(
    fake: &FamilyFake,
    method: &Method,
    segments: &[&str],
    body: &[u8],
) -> Option<Response> {
    match (method, segments) {
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
