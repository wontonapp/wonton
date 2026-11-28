//! 按角色开放应用（ADR-0088）：列表、详情、网页路由一起受限，改角色立即生效。

use hyper::{Method, StatusCode};
use iroh::SecretKey;
use serde_json::json;

use crate::gateway::family_tests::{PORT, bearer, send_as};
use crate::gateway::tests::{CORE, Harness, PLUGIN, WHOAMI};
use crate::gateway::{APP_COOKIE, Person};
use crate::plugin::PluginPackage;
use crate::plugin::languages::Languages;

use super::invite_record;

#[tokio::test]
async fn selected_apps_limit_lists_details_and_webui_even_for_installers() {
    let h = Harness::start(None).await;
    let device = SecretKey::generate().public();
    invite_record(&h, false).await;
    h.tunnel
        .devices
        .add_if(device, None, Some("mia".into()), |_| true)
        .unwrap();
    let other = WHOAMI.replace("app.whoami", "app.other");
    h.manager.register_for_test(
        PluginPackage::manifest_of(&other, &Languages::default()).unwrap(),
        crate::plugin::PluginStatus::Running,
        Some(h.upstream.to_string()),
    );
    let draft = wonton_protocol::RoleDraft {
        name: "选中的应用".into(),
        permissions: wonton_protocol::Permissions {
            open_apps: wonton_protocol::AppScope::Selected {
                plugin_ids: vec!["app.whoami".into()],
            },
            install_apps: wonton_protocol::Gate::Allow,
            ..wonton_protocol::Permissions::strictest()
        },
    };
    let role = h.roles.create(&draft).unwrap();
    h.members.set_role("mia", &role.id).unwrap();
    let reply = send_as(
        &h.app,
        Some(device),
        Method::GET,
        CORE,
        "/_wonton/api/plugins",
        &[],
        "",
    )
    .await;
    let ids: Vec<_> = reply.json()["plugins"]
        .as_array()
        .unwrap()
        .iter()
        .map(|plugin| plugin["id"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(ids, ["app.whoami"]);
    assert_opened(&h, device).await;
    let hits = h.hits.load(std::sync::atomic::Ordering::SeqCst);
    for (host, path) in [
        (CORE, "/_wonton/api/plugins/app.other"),
        ("app-other.wonton.localhost", "/hello"),
    ] {
        let reply = send_as(&h.app, Some(device), Method::GET, host, path, &[], "").await;
        assert_eq!(reply.status, StatusCode::FORBIDDEN, "{}", reply.text());
        assert_eq!(reply.error_kind(), Some("not_shared"));
    }
    assert_eq!(h.hits.load(std::sync::atomic::Ordering::SeqCst), hits);
    let mut draft = draft;
    draft.permissions.open_apps = wonton_protocol::AppScope::Other;
    h.roles.update(&role.id, &draft).unwrap();
    assert_gated(&h, device, Some("not_shared")).await;
    let owner = send_as(
        &h.app,
        None,
        Method::GET,
        CORE,
        "/_wonton/api/plugins",
        &[("authorization", bearer().as_str())],
        "",
    )
    .await;
    let mut owner_ids: Vec<_> = owner.json()["plugins"]
        .as_array()
        .unwrap()
        .iter()
        .map(|plugin| plugin["id"].as_str().unwrap().to_owned())
        .collect();
    owner_ids.sort();
    assert_eq!(owner_ids, ["app.other", "app.whoami"]);
}

#[tokio::test]
async fn role_changes_override_the_family_switch_and_recheck_existing_web_sessions() {
    let h = Harness::start(None).await;
    let device = SecretKey::generate().public();
    invite_record(&h, false).await;
    h.tunnel
        .devices
        .add_if(device, None, Some("mia".into()), |_| true)
        .unwrap();
    write_whoami_record(&h);
    let reply = send_as(
        &h.app,
        None,
        Method::PUT,
        CORE,
        "/_wonton/api/plugins/app.whoami/audience",
        &[
            ("authorization", bearer().as_str()),
            ("content-type", "application/json"),
        ],
        r#"{"role_ids": ["admin", "family"]}"#,
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    assert_gated(&h, device, Some("not_shared")).await;
    let sid = h.gateway.sessions.create_app(
        &Person {
            username: "mia".into(),
            role: wonton_protocol::Role::Member,
        },
        PLUGIN,
    );
    let cookie = format!("{}={sid}", APP_COOKIE);
    let mut draft = wonton_protocol::RoleDraft {
        name: "朋友".into(),
        permissions: wonton_protocol::Permissions {
            open_apps: wonton_protocol::AppScope::All,
            ..wonton_protocol::Permissions::strictest()
        },
    };
    h.roles.update("friend", &draft).unwrap();
    let reply = h.get(PLUGIN, "/hello?x=1", &[("cookie", &cookie)]).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    assert_eq!(reply.json()["path"], "/hello?x=1");
    draft.permissions.open_apps = wonton_protocol::AppScope::Selected { plugin_ids: vec![] };
    h.roles.update("friend", &draft).unwrap();
    let hits = h.hits.load(std::sync::atomic::Ordering::SeqCst);
    let reply = h.get(PLUGIN, "/hello", &[("cookie", &cookie)]).await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN, "{}", reply.text());
    assert_eq!(reply.error_kind(), Some("not_shared"));
    assert_eq!(h.hits.load(std::sync::atomic::Ordering::SeqCst), hits);
}

/// 详情和网页放行，网页的请求转发到插件。
async fn assert_opened(h: &Harness, hers: iroh::EndpointId) {
    let reply = send_as(
        &h.app,
        Some(hers),
        Method::GET,
        CORE,
        "/_wonton/api/plugins/app.whoami",
        &[],
        "",
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    let reply = send_as(
        &h.app,
        Some(hers),
        Method::GET,
        PLUGIN,
        "/hello?x=1",
        &[],
        "",
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    assert_eq!(reply.json()["path"], "/hello?x=1");
    assert_eq!(reply.json()["headers"]["host"], format!("{PLUGIN}:{PORT}"));
}

/// 角色没开放应用：列表为空，详情和网页都回 403，错误码是 `kind`。
async fn assert_gated(h: &Harness, hers: iroh::EndpointId, kind: Option<&'static str>) {
    let reply = send_as(
        &h.app,
        Some(hers),
        Method::GET,
        CORE,
        "/_wonton/api/plugins",
        &[],
        "",
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    assert!(
        reply.json()["plugins"].as_array().unwrap().is_empty(),
        "{}",
        reply.text()
    );
    let reply = send_as(
        &h.app,
        Some(hers),
        Method::GET,
        CORE,
        "/_wonton/api/plugins/app.whoami",
        &[],
        "",
    )
    .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert_eq!(reply.error_kind(), kind);
    let reply = send_as(
        &h.app,
        Some(hers),
        Method::GET,
        PLUGIN,
        "/hello?x=1",
        &[],
        "",
    )
    .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN, "{}", reply.text());
    assert_eq!(reply.error_kind(), kind);

    // 接口拿到的体是 JSON：看得见是网关拦的、为什么拦，不是一页 HTML 解析不了。
    let reply = send_as(
        &h.app,
        Some(hers),
        Method::GET,
        PLUGIN,
        "/api/data",
        &[("accept", "*/*")],
        "",
    )
    .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN, "{}", reply.text());
    assert_eq!(reply.error_kind(), kind);
    assert_eq!(reply.content_type(), "application/json");
    assert_eq!(reply.json()["error"]["category"], "not_shared");

    // 同一个人从浏览器打开：还是 HTML 错误页（已经登录，不跳登录页）。
    let reply = send_as(
        &h.app,
        Some(hers),
        Method::GET,
        PLUGIN,
        "/hello?x=1",
        &[("accept", "text/html")],
        "",
    )
    .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN, "{}", reply.text());
    assert_eq!(reply.error_kind(), kind);
    assert!(
        reply.text().starts_with("<!doctype html>"),
        "{}",
        reply.text()
    );
}

/// 家人测试要的插件记录：register_for_test 只填内存表，开关和详情要读记录。
fn write_whoami_record(h: &Harness) {
    std::fs::create_dir_all(h.data_dir.join("plugins")).unwrap();
    std::fs::write(
        h.data_dir.join("plugins").join("app.whoami.json"),
        json!({
            "id": "app.whoami",
            "version": "1.0.0",
            "source": "services: {}",
            "granted_permissions": [],
            "installed_at": 1,
        })
        .to_string(),
    )
    .unwrap();
}
