//! 给插件的只有能力，不是身份组（ADR-0088 第 8 节）：主人和“在这个应用里当管理员”的人登同一个
//! OIDC 客户端，`groups` 里是 `plugin_admin` 加清单声明的组、`role_claim` 写管理员值；普通家人、
//! 管理员范围圈不到的应用，都只有通用的 `user`。

use std::sync::{Arc, Mutex};

use hyper::{Method, StatusCode};
use iroh::SecretKey;
use serde_json::{Value, json};
use wonton_protocol::{AdminScope, AppScope, MemberStatus, Permissions, RoleDraft};

use super::CORE_COOKIE;
use super::fakes::fake_hydra_subject_follows;
use super::family_tests::send_as;
use super::tests::{CORE, Harness, TOKEN, WHOAMI};
use crate::members::MemberRecord;
use crate::plugin::languages::Languages;
use crate::plugin::{PluginPackage, PluginStatus};

async fn start(subject: Arc<Mutex<String>>) -> (Arc<Mutex<Vec<Value>>>, Harness) {
    let accepted = Arc::new(Mutex::new(Vec::new()));
    let hydra = fake_hydra_subject_follows(accepted.clone(), subject).await;
    let h = Harness::start(Some(hydra)).await;
    // 声明了管理员组和角色声明的客户端：challenge 兼作客户端 ID（假 Hydra 的约定）。
    let books = PluginPackage::manifest_of(
        &WHOAMI.replace("app.whoami", "app.books").replace(
            "  webui:\n    service: web\n    port: 8000",
            "  webui:\n    service: web\n    port: 8000\n    sso: oidc\n    oidc:\n      \
                 redirect_paths: [/cb]\n      admin_groups: [admin, app.admin]\n      \
                 role_claim: {name: immich_role, admin: admin, user: user}",
        ),
        &Languages::from_header(Some("zh-Hans")),
    )
    .unwrap();
    assert_eq!(books.id.as_str(), "app.books");
    h.manager
        .register_for_test(books, PluginStatus::Running, None);
    (accepted, h)
}

/// 记一个加入了的人并给他配上设备。
fn member(h: &Harness, username: &str, role_id: &str) -> iroh::EndpointId {
    h.members
        .invite(MemberRecord {
            role_id: role_id.to_owned(),
            status: MemberStatus::Joined,
            ..MemberRecord::for_test(username)
        })
        .unwrap();
    let id = SecretKey::generate().public();
    h.tunnel
        .devices
        .add_if(id, None, Some(username.to_owned()), |_| true)
        .unwrap();
    id
}

fn claims_of(accepted: &Mutex<Vec<Value>>, nth: usize) -> Value {
    accepted.lock().unwrap()[nth]["session"]["id_token"].clone()
}

#[tokio::test]
async fn claims_carry_plugin_admin_only_where_the_role_grants_it() {
    let subject = Arc::new(Mutex::new("owner".to_owned()));
    let (accepted, h) = start(subject.clone()).await;

    // 主人：`plugin_admin` 加清单的组和声明。
    let core = h.login(TOKEN).await.cookie(CORE_COOKIE);
    let reply = send_as(
        &h.app,
        None,
        Method::GET,
        CORE,
        "/_wonton/oidc/consent?consent_challenge=app.books",
        &[("cookie", core.as_str())],
        "",
    )
    .await;
    assert_eq!(reply.status, StatusCode::SEE_OTHER, "{}", reply.text());

    // “管理员”模板的成员：同一个客户端，同样拿到能力和应用自己的组。
    let ada = member(&h, "ada", "admin");
    *subject.lock().unwrap() = "ada".to_owned();
    let reply = send_as(
        &h.app,
        Some(ada),
        Method::GET,
        CORE,
        "/_wonton/oidc/consent?consent_challenge=app.books",
        &[],
        "",
    )
    .await;
    assert_eq!(reply.status, StatusCode::SEE_OTHER, "{}", reply.text());

    // 家人模板：应用没开放给他，只有通用的 `user`，声明写普通值。
    let ben = member(&h, "ben", "family");
    *subject.lock().unwrap() = "ben".to_owned();
    let reply = send_as(
        &h.app,
        Some(ben),
        Method::GET,
        CORE,
        "/_wonton/oidc/consent?consent_challenge=app.books",
        &[],
        "",
    )
    .await;
    assert_eq!(reply.status, StatusCode::SEE_OTHER, "{}", reply.text());

    // 勾了在这个应用当管理员、但应用没开放给他：管理员范围不生效（计划第 4.13 条）。
    let caro_role = h
        .roles
        .create(&RoleDraft {
            name: "只圈不开放".into(),
            permissions: Permissions {
                open_apps: AppScope::Selected {
                    plugin_ids: vec!["app.other".into()],
                },
                plugin_admin: AdminScope::Selected {
                    plugin_ids: vec!["app.books".into()],
                },
                ..Permissions::strictest()
            },
        })
        .unwrap();
    let caro = member(&h, "caro", &caro_role.id);
    *subject.lock().unwrap() = "caro".to_owned();
    let reply = send_as(
        &h.app,
        Some(caro),
        Method::GET,
        CORE,
        "/_wonton/oidc/consent?consent_challenge=app.books",
        &[],
        "",
    )
    .await;
    assert_eq!(reply.status, StatusCode::SEE_OTHER, "{}", reply.text());

    assert_eq!(accepted.lock().unwrap().len(), 4);
    for (nth, username) in [(0, "owner"), (1, "ada")] {
        let claims = claims_of(&accepted, nth);
        assert_eq!(claims["preferred_username"], username);
        assert_eq!(
            claims["groups"],
            json!(["user", "plugin_admin", "admin", "app.admin"]),
            "{username}"
        );
        assert_eq!(claims["immich_role"], "admin", "{username}");
    }
    for (nth, username) in [(2, "ben"), (3, "caro")] {
        let claims = claims_of(&accepted, nth);
        assert_eq!(claims["preferred_username"], username);
        assert_eq!(claims["groups"], json!(["user"]), "{username}");
        assert_eq!(claims["immich_role"], "user", "{username}");
    }
}
