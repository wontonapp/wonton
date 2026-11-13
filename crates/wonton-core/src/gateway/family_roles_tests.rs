//! 角色（ADR-0088）在网关上的行为：邀请带角色、`Me` 照实报角色和权限、角色删不掉的情况、
//! 不认识的取值按最严。家人自己的流程在 `family_tests`，这里只用它的夹具。

use hyper::{Method, StatusCode};
use iroh::SecretKey;
use serde_json::{Value, json};

use super::family_tests::{
    ME, MEMBERS, home_with_matrix, invite, owner_call, seed_member, send_as,
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
    assert_eq!(reply.error_kind(), Some("install_not_allowed"));
}

/// 旧的“能装插件”开关不再写记录：回 409 说清楚改由角色决定，人不在时照旧 404。
#[tokio::test]
async fn the_old_install_switch_says_that_roles_decide_now() {
    let (home, _matrix_state) = home_with_matrix().await;
    let member = invite(
        &home,
        json!({"display_name": "Kid", "private_bot": false, "role_id": "child"}),
    )
    .await;
    assert_eq!(member["role_id"], "child");
    let (status, reply) = owner_call(
        &home,
        Method::PUT,
        &wonton_protocol::member_path("kid"),
        json!({"can_install_plugins": true}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{reply}");
    assert_eq!(reply["error"]["kind"], "install_apps_from_role", "{reply}");
    assert_eq!(
        home.members.get("kid").unwrap().role_id,
        "child",
        "只回错误，不动角色"
    );

    let (status, reply) = owner_call(
        &home,
        Method::PUT,
        &wonton_protocol::member_path("nobody"),
        json!({"can_install_plugins": true}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{reply}");
}
