//! 按权限项分层的路由：缺项回 403 带缺的项、没身份回 401，装了应用就能连权限项一起改。

use hyper::{Method, StatusCode};
use iroh::SecretKey;
use serde_json::json;

use crate::gateway::tests::{CORE, Harness};

use super::support::*;

/// 按权限项分层的路由表：每一项下，没有这一项的家人拿到的都是 403 和缺的那一项。
type GuardedRoutes = Vec<(
    wonton_protocol::PermissionItem,
    Vec<(Method, &'static str, &'static str)>,
)>;

fn install_guarded_routes() -> Vec<(Method, &'static str, &'static str)> {
    vec![
        (
            Method::POST,
            "/_wonton/api/plugins/preview",
            r#"{"source":"github:o/r"}"#,
        ),
        (Method::POST, "/_wonton/api/plugins", "{}"),
        (Method::POST, "/_wonton/api/plugins/app.whoami/start", ""),
        (Method::GET, "/_wonton/api/plugins/app.whoami/lifecycle", ""),
        (Method::POST, "/_wonton/api/plugins/app.whoami/stop", ""),
        (
            Method::POST,
            "/_wonton/api/plugins/app.whoami/upgrade",
            "{}",
        ),
        (Method::DELETE, "/_wonton/api/plugins/app.whoami", ""),
        (Method::GET, "/_wonton/api/store", ""),
        (
            Method::GET,
            "/_wonton/api/store/plugin?source=github:o/r",
            "",
        ),
        (
            Method::PUT,
            "/_wonton/api/plugins/app.whoami/permissions/docker.socket",
            "{\"enabled\": true}",
        ),
        (
            Method::GET,
            "/_wonton/api/plugins/app.whoami/tool-policy",
            "",
        ),
        (
            Method::PUT,
            "/_wonton/api/plugins/app.whoami/tool-policy/always/hermes",
            "",
        ),
        (Method::GET, "/_wonton/api/plugins/app.whoami/secrets", ""),
        (
            Method::PUT,
            "/_wonton/api/plugins/app.whoami/secrets/admin_password",
            "{\"value\": \"x\"}",
        ),
        (Method::PUT, "/_wonton/api/plugins/app.whoami/takeover", ""),
        (Method::PUT, "/_wonton/api/bots/app.whoami/profile", "{}"),
        (
            Method::PUT,
            "/_wonton/api/plugins/app.whoami/scope",
            "{\"type\":\"all\"}",
        ),
        (
            Method::GET,
            "/_wonton/api/bots/app.agent/default/plugin-access/app.whoami",
            "",
        ),
    ]
}

fn guarded_routes() -> GuardedRoutes {
    use wonton_protocol::PermissionItem as Item;
    let install = install_guarded_routes();
    let server: Vec<(Method, &str, &str)> = vec![
        (Method::GET, "/_wonton/api/egress-proxy", ""),
        (Method::PUT, "/_wonton/api/egress-proxy", "{}"),
        (Method::GET, "/_wonton/api/registry-mirror", ""),
        (Method::POST, "/_wonton/api/registry-mirror/test", "{}"),
        (Method::GET, "/_wonton/api/lan", ""),
        (Method::GET, "/_wonton/api/onboarding", ""),
        (Method::POST, "/_wonton/api/core/upgrade/check", ""),
        (Method::POST, "/_wonton/api/bots", "{}"),
        (Method::GET, "/_wonton/api/storage", ""),
        (Method::POST, "/_wonton/api/store/sources", "{}"),
        (Method::DELETE, "/_wonton/api/store/sources?address=x", ""),
        (Method::GET, "/_wonton/api/peers", ""),
        (Method::POST, "/_wonton/api/peers/requests/r1/approve", ""),
    ];
    vec![
        (Item::ServerSettings, server),
        (
            Item::ManageMembers,
            vec![
                (Method::GET, MEMBERS, ""),
                (Method::POST, MEMBERS, "{}"),
                (Method::POST, "/_wonton/api/members/mia/pairing-code", "{}"),
                (Method::POST, "/_wonton/api/members/mia/remove", "{}"),
            ],
        ),
        (Item::InstallApps, install),
        (
            Item::Owner,
            vec![(
                Method::PUT,
                "/_wonton/api/plugins/app.whoami/audience",
                "{\"role_ids\": [\"admin\"]}",
            )],
        ),
        (
            Item::ModelSettings,
            vec![(Method::GET, "/_wonton/api/models/settings", "")],
        ),
    ]
}

/// 每一项：没有这一项的家人是 403 并带上缺的项，没有身份是 401；换成管理员模板就放行，角色接口仍只给主人。
#[tokio::test]
async fn routes_ask_for_their_permission_item() {
    let h = Harness::start(None).await;
    let member = SecretKey::generate().public();
    seed_member(&h, "mia", false);
    h.tunnel
        .devices
        .add_if(member, None, Some("mia".to_owned()), |_| true)
        .unwrap();

    for (item, routes) in guarded_routes() {
        for (method, path, body) in routes {
            let reply = send_as(&h.app, Some(member), method.clone(), CORE, path, &[], body).await;
            assert_eq!(
                reply.status,
                StatusCode::FORBIDDEN,
                "{item:?} {method} {path}: {}",
                reply.text()
            );
            assert_eq!(reply.error_kind(), Some("permission_required"));
            assert_eq!(reply.json()["missing"], json!(item), "{path}");
            // 同一条路由没有身份时是 401。
            let anonymous = send_as(&h.app, None, method.clone(), CORE, path, &[], body).await;
            assert_eq!(
                anonymous.status,
                StatusCode::UNAUTHORIZED,
                "{method} {path}: {}",
                anonymous.text()
            );
            assert_eq!(anonymous.error_kind(), Some("unauthenticated"));
        }
    }

    // 管理员模板：成员、设备、服务器设置都放行；角色接口仍然只给主人。
    h.members.set_role("mia", "admin").unwrap();
    for (method, path, body) in [
        (Method::GET, MEMBERS, ""),
        (Method::GET, "/_wonton/api/devices", ""),
        (Method::GET, "/_wonton/api/storage", ""),
        (Method::POST, "/_wonton/api/store/sources", "{}"),
        (Method::POST, "/_wonton/api/plugins/app.whoami/start", ""),
        (Method::GET, "/_wonton/api/plugins/app.whoami/lifecycle", ""),
        (Method::POST, "/_wonton/api/plugins/app.whoami/stop", ""),
    ] {
        let reply = send_as(&h.app, Some(member), method.clone(), CORE, path, &[], body).await;
        assert_ne!(
            reply.status,
            StatusCode::FORBIDDEN,
            "管理员 {method} {path}: {}",
            reply.text()
        );
    }
    // 三个接口复用启动权限；未安装只报业务错误，不能把获授权的家人误挡为未登录或仅限主人。
    for (method, path) in [
        (Method::POST, "/_wonton/api/plugins/app.missing/start"),
        (Method::GET, "/_wonton/api/plugins/app.missing/lifecycle"),
        (Method::POST, "/_wonton/api/plugins/app.missing/stop"),
    ] {
        let reply = send_as(&h.app, Some(member), method.clone(), CORE, path, &[], "").await;
        assert_eq!(
            reply.status,
            StatusCode::NOT_FOUND,
            "{method} {path}: {}",
            reply.text()
        );
        assert_eq!(reply.error_kind(), Some("not_installed"), "{path}");
    }
    let reply = send_as(
        &h.app,
        Some(member),
        Method::GET,
        CORE,
        wonton_protocol::ROLES_PATH,
        &[],
        "",
    )
    .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN, "{}", reply.text());
    assert_eq!(reply.json()["missing"], json!("owner"));

    // 登记设备只认主人令牌（ADR-0009）：家人的设备也是 401 unauthenticated，不是 permission_required。
    let reply = send_as(
        &h.app,
        Some(member),
        Method::POST,
        CORE,
        "/_wonton/api/devices",
        &[("content-type", "application/json")],
        "{\"endpoint_id\": \"zz\"}",
    )
    .await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED, "{}", reply.text());
    assert_eq!(reply.error_kind(), Some("unauthenticated"));
}

/// 有“装、删、升级应用”就能连权限项一起改（ADR-0088 把插件的设置和权限都归这一项）。
#[tokio::test]
async fn installers_can_change_plugin_permissions() {
    let h = Harness::start(None).await;
    let member = SecretKey::generate().public();
    seed_member(&h, "mia", true);
    h.tunnel
        .devices
        .add_if(member, None, Some("mia".to_owned()), |_| true)
        .unwrap();

    for (method, path, content_type, body) in [
        (
            Method::POST,
            "/_wonton/api/plugins",
            "application/json",
            r#"{"source":"github:o/r","grant":[],"enabled_permissions":["docker.socket"]}"#,
        ),
        (
            Method::POST,
            "/_wonton/api/plugins/app.whoami/upgrade",
            "application/json",
            r#"{"grant":[],"enabled_permissions":["docker.socket"]}"#,
        ),
        (
            Method::POST,
            "/_wonton/api/plugins?enabled_permissions=docker.socket",
            "application/yaml",
            "{}",
        ),
        (
            Method::POST,
            "/_wonton/api/plugins/app.whoami/upgrade?enabled_permissions=docker.socket",
            "application/yaml",
            "{}",
        ),
        (
            Method::PUT,
            "/_wonton/api/plugins/app.whoami/permissions/docker.socket",
            "application/json",
            r#"{"enabled":true}"#,
        ),
        (
            Method::PUT,
            "/_wonton/api/plugins/app.whoami/permissions/docker.socket",
            "application/json",
            r#"{"enabled":false}"#,
        ),
    ] {
        let reply = send_as(
            &h.app,
            Some(member),
            method.clone(),
            CORE,
            path,
            &[("content-type", content_type)],
            body,
        )
        .await;
        assert_ne!(
            reply.status,
            StatusCode::FORBIDDEN,
            "{method} {path}: {}",
            reply.text()
        );
    }
}
