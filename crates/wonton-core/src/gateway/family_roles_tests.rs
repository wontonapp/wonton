//! 角色（ADR-0088）在网关上的行为：邀请带角色、`Me` 照实报角色和权限、角色删不掉的情况、
//! 不认识的取值按最严。家人自己的流程在 `family_tests`，这里只用它的夹具。

use hyper::{Method, StatusCode};
use iroh::SecretKey;
use serde_json::{Value, json};

use super::family_tests::{
    ME, MEMBERS, bearer, home_with_matrix, invite, owner_call, seed_member, send_as,
};
use super::tests::{CORE, Harness};

fn role_draft(name: &str) -> wonton_protocol::RoleDraft {
    wonton_protocol::RoleDraft {
        name: name.to_owned(),
        permissions: wonton_protocol::Permissions::strictest(),
    }
}

/// 主人邀请时能选角色：记的是选的那个，被邀请的人 `Me` 里照实给出角色和权限。
#[tokio::test]
async fn an_invite_picks_a_role_and_me_reports_it() {
    let (home, _matrix_state) = home_with_matrix().await;
    let member = invite(
        &home,
        json!({"display_name": "Kid", "private_bot": false, "role_id": "child"}),
    )
    .await;
    assert_eq!(member["role_id"], "child");

    let device = home.member_device("kid");
    let reply = send_as(&home.app, Some(device), Method::GET, CORE, ME, &[], "").await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    let me: Value = serde_json::from_str(reply.text()).unwrap();
    assert_eq!(me["is_owner"], false);
    assert_eq!(me["role_id"], "child");
    assert_eq!(me["role_name"], Value::Null, "模板名由壳本地化");
    assert_eq!(me["permissions"]["install_apps"], "ask_admin");
    assert_eq!(me["permissions"]["ai"], "admin_approves_deletes");
    assert_eq!(me["permissions"]["private_bot"], true);
}

/// 不传角色时记默认邀请角色（初始是“朋友”），不按主人的身份凑一个。
#[tokio::test]
async fn an_invite_without_a_role_gets_the_default_invite_role() {
    let (home, _matrix_state) = home_with_matrix().await;
    let member = invite(
        &home,
        json!({"display_name": "Guest", "private_bot": false}),
    )
    .await;
    assert_eq!(member["role_id"], "friend");
    assert_eq!(home.members.get("guest").unwrap().role_id, "friend");
}

/// 要私人 Bot 时角色必须有私人 Bot：朋友模板没有，家人模板有。
#[tokio::test]
async fn a_private_bot_needs_a_role_that_has_one() {
    let (home, _matrix_state) = home_with_matrix().await;
    for mut body in [
        json!({"display_name": "Guest", "private_bot": true, "role_id": "friend"}),
        json!({"display_name": "Guest", "private_bot": true}),
    ] {
        let (status, reply) = owner_call(&home, Method::POST, MEMBERS, body.take()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{reply}");
        assert_eq!(reply["error"]["kind"], "invalid_input", "{reply}");
    }
    let (status, reply) = owner_call(
        &home,
        Method::POST,
        MEMBERS,
        json!({"display_name": "Guest", "private_bot": true, "role_id": "family"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{reply}");
    assert_eq!(reply["role_id"], "family");
}

/// 没有这个角色：邀请回 400，不按默认静默落下去。
#[tokio::test]
async fn an_unknown_role_is_refused() {
    let (home, _matrix_state) = home_with_matrix().await;
    let (status, reply) = owner_call(
        &home,
        Method::POST,
        MEMBERS,
        json!({"display_name": "Guest", "private_bot": false, "role_id": "nobody"}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{reply}");
    assert_eq!(reply["error"]["kind"], "invalid_input", "{reply}");
}

/// 还有人在用的角色、以及默认邀请角色删不掉；没人用了才删得掉。人数按没移除的家人和好友算。
#[tokio::test]
async fn a_role_in_use_and_the_default_one_cannot_be_deleted() {
    let h = Harness::start(None).await;
    let role = h.roles.create(&role_draft("客人")).unwrap();
    seed_member(&h, "mia", false);
    h.members.set_role("mia", &role.id).unwrap();
    assert_eq!(h.gateway.role_people(&role.id), 1, "在用的人计进人数");
    assert_eq!(h.gateway.role_people("family"), 0);
    assert!(matches!(
        h.gateway.delete_role(&role.id),
        Err(crate::roles::RolesError::InUse(_))
    ));
    assert!(matches!(
        h.gateway.delete_role("friend"),
        Err(crate::roles::RolesError::DefaultRole)
    ));

    // 移掉这个人、换成别的角色，就能删了。
    h.members.remove("mia", false, true).unwrap();
    assert_eq!(h.gateway.role_people(&role.id), 0, "移除的人不再算");
    h.gateway.delete_role(&role.id).unwrap();
    assert!(h.roles.get(&role.id).is_none());
}

/// 角色里的权限取值认不出时按最严的处理：安装类接口一律挡下，`Me` 里照实报 `other`。
#[tokio::test]
async fn an_unknown_permission_value_reads_as_the_strictest() {
    let h = Harness::start(None).await;
    let role = h
        .roles
        .create(&wonton_protocol::RoleDraft {
            name: "怪".to_owned(),
            permissions: wonton_protocol::Permissions {
                install_apps: wonton_protocol::Gate::Other,
                ..wonton_protocol::Permissions::strictest()
            },
        })
        .unwrap();
    seed_member(&h, "mia", false);
    h.members.set_role("mia", &role.id).unwrap();
    let device = SecretKey::generate().public();
    h.tunnel
        .devices
        .add_if(device, None, Some("mia".to_owned()), |_| true)
        .unwrap();

    let reply = send_as(&h.app, Some(device), Method::GET, CORE, ME, &[], "").await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    let me: Value = serde_json::from_str(reply.text()).unwrap();
    assert_eq!(me["permissions"]["install_apps"], "other");

    let reply = send_as(
        &h.app,
        Some(device),
        Method::DELETE,
        CORE,
        "/_wonton/api/plugins/app.whoami",
        &[],
        "",
    )
    .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN, "{}", reply.text());
    assert_eq!(reply.error_kind(), Some("permission_required"));
    assert_eq!(reply.json()["missing"], "install_apps");
}

/// 分配角色以后，下一次请求 `Me` 就是新角色和它的权限。
#[tokio::test]
async fn an_assignment_shows_up_in_the_next_me() {
    let (home, _matrix_state) = home_with_matrix().await;
    invite(
        &home,
        json!({"display_name": "Kid", "private_bot": true, "role_id": "child"}),
    )
    .await;
    let device = home.member_device("kid");

    let reply = assign(&home, "kid", "admin").await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    let list: Value = serde_json::from_str(reply.text()).unwrap();
    let admin = list["roles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|role| role["id"] == "admin")
        .unwrap();
    assert_eq!(admin["people"], 1, "人数跟着变");

    let reply = send_as(&home.app, Some(device), Method::GET, CORE, ME, &[], "").await;
    let me: Value = serde_json::from_str(reply.text()).unwrap();
    assert_eq!(me["role_id"], "admin");
    assert_eq!(me["permissions"]["install_apps"], "allow");
}

/// 给一个家人分配角色，回响应。
async fn assign(home: &super::tunnel_tests::Home, who: &str, role_id: &str) -> super::tests::Reply {
    send_as(
        &home.app,
        None,
        Method::PUT,
        CORE,
        &wonton_protocol::member_role_path(who),
        &[
            ("authorization", bearer().as_str()),
            ("content-type", "application/json"),
        ],
        &format!("{{\"role_id\": \"{role_id}\"}}"),
    )
    .await
}

/// 给不在记录里、或者已经移除的人分配：404 和 409 分开报；角色本身不存在时 404。
#[tokio::test]
async fn assigning_refuses_unknown_and_removed_people() {
    let (home, _matrix_state) = home_with_matrix().await;
    invite(
        &home,
        json!({"display_name": "Kid", "private_bot": false, "role_id": "child"}),
    )
    .await;
    let reply = assign(&home, "nobody", "family").await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND, "{}", reply.text());

    home.members.remove("kid", false, true).unwrap();
    let reply = assign(&home, "kid", "family").await;
    assert_eq!(reply.status, StatusCode::CONFLICT, "{}", reply.text());
    assert_eq!(reply.error_kind(), Some("removed"));

    let reply = assign(&home, "kid", "nobody").await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND, "{}", reply.text());
    assert_eq!(reply.error_kind(), Some("role_not_found"));
}

/// 主人令牌发一条请求。
async fn owner(h: &Harness, method: Method, path: &str, body: &str) -> super::tests::Reply {
    send_as(
        &h.app,
        None,
        method,
        CORE,
        path,
        &[
            ("authorization", bearer().as_str()),
            ("content-type", "application/json"),
        ],
        body,
    )
    .await
}

/// 建、改走一遍：新建给 ID，重名 409，改名和改权限都生效。
#[tokio::test]
async fn roles_are_created_and_renamed_and_names_must_differ() {
    let h = Harness::start(None).await;
    let reply = owner(
        &h,
        Method::POST,
        wonton_protocol::ROLES_PATH,
        "{\"name\": \"客人\", \"permissions\": {}}",
    )
    .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.text());
    let created: Value = serde_json::from_str(reply.text()).unwrap();
    let id = created["id"].as_str().unwrap().to_owned();
    assert_eq!(created["people"], 0);
    assert_eq!(
        created["permissions"]["install_apps"], "deny",
        "没写的项按最严"
    );

    let reply = owner(
        &h,
        Method::POST,
        wonton_protocol::ROLES_PATH,
        "{\"name\": \" 客人 \", \"permissions\": {}}",
    )
    .await;
    assert_eq!(reply.status, StatusCode::CONFLICT, "{}", reply.text());
    assert_eq!(reply.error_kind(), Some("role_name_taken"));

    let reply = owner(
        &h,
        Method::PUT,
        &wonton_protocol::role_path(&id),
        "{\"name\": \"常客\", \"permissions\": {\"install_apps\": \"allow\"}}",
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    let updated: Value = serde_json::from_str(reply.text()).unwrap();
    assert_eq!(updated["name"], "常客");
    assert_eq!(updated["permissions"]["install_apps"], "allow");

    let reply = owner(
        &h,
        Method::PUT,
        &wonton_protocol::role_path("nobody"),
        "{\"name\": \"X\", \"permissions\": {}}",
    )
    .await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND, "{}", reply.text());
    assert_eq!(reply.error_kind(), Some("role_not_found"));
}

/// 删：默认角色和在用的删不掉，没人在用的删得掉；家人设备的每个接口都是缺“主人”的 403。
#[tokio::test]
async fn deleting_roles_checks_default_and_people_and_members_are_refused() {
    let h = Harness::start(None).await;
    let spare = h.roles.create(&role_draft("备用")).unwrap();
    seed_member(&h, "mia", false);
    h.members.set_role("mia", &spare.id).unwrap();

    let reply = owner(
        &h,
        Method::DELETE,
        &wonton_protocol::role_path("friend"),
        "",
    )
    .await;
    assert_eq!(reply.status, StatusCode::CONFLICT, "{}", reply.text());
    assert_eq!(reply.error_kind(), Some("default_role"));

    let reply = owner(
        &h,
        Method::DELETE,
        &wonton_protocol::role_path(&spare.id),
        "",
    )
    .await;
    assert_eq!(reply.status, StatusCode::CONFLICT, "{}", reply.text());
    assert_eq!(reply.error_kind(), Some("role_in_use"));

    // 家人设备的每个接口都是缺“主人”的 403。
    let device = SecretKey::generate().public();
    h.tunnel
        .devices
        .add_if(device, None, Some("mia".to_owned()), |_| true)
        .unwrap();
    for (method, path, body) in [
        (Method::GET, wonton_protocol::ROLES_PATH.to_owned(), ""),
        (Method::POST, wonton_protocol::ROLES_PATH.to_owned(), "{}"),
        (Method::PUT, wonton_protocol::role_path("admin"), "{}"),
        (Method::DELETE, wonton_protocol::role_path("admin"), ""),
        (
            Method::PUT,
            wonton_protocol::DEFAULT_INVITE_ROLE_PATH.to_owned(),
            "{}",
        ),
        (Method::PUT, wonton_protocol::member_role_path("mia"), "{}"),
        (
            Method::PUT,
            wonton_protocol::friend_role_path("@a:test"),
            "{}",
        ),
    ] {
        let reply = send_as(&h.app, Some(device), method, CORE, &path, &[], body).await;
        assert_eq!(
            reply.status,
            StatusCode::FORBIDDEN,
            "{path}: {}",
            reply.text()
        );
        assert_eq!(reply.error_kind(), Some("permission_required"), "{path}");
        let body: Value = serde_json::from_str(reply.text()).unwrap();
        assert_eq!(body["missing"], "owner", "{path}");
    }

    // 没人用的删掉：204。
    h.members.remove("mia", false, true).unwrap();
    let reply = owner(
        &h,
        Method::DELETE,
        &wonton_protocol::role_path(&spare.id),
        "",
    )
    .await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT, "{}", reply.text());
}

/// 换默认邀请角色：回全部角色，列表里默认跟着变；角色不存在 404。
#[tokio::test]
async fn the_default_invite_role_can_be_changed_through_the_api() {
    let h = Harness::start(None).await;
    let reply = owner(
        &h,
        Method::PUT,
        wonton_protocol::DEFAULT_INVITE_ROLE_PATH,
        "{\"role_id\": \"child\"}",
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    let list: Value = serde_json::from_str(reply.text()).unwrap();
    assert_eq!(list["default_invite_role"], "child");

    let reply = owner(
        &h,
        Method::PUT,
        wonton_protocol::DEFAULT_INVITE_ROLE_PATH,
        "{\"role_id\": \"nobody\"}",
    )
    .await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND, "{}", reply.text());
}
