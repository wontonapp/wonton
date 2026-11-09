//! 家人和插件：开放给家人（`plugin_family_path`）、确认卡片归属、OIDC 声明按当前人（ADR-0045）。
//! 共用的搭建在 [`super::family_tests`]。

use std::sync::{Arc, Mutex};

use hyper::{Method, StatusCode};
use iroh::SecretKey;
use serde_json::{Value, json};

use super::fakes::fake_hydra_for;
use super::family_tests::{PORT, bearer, send_as};
use super::tests::{CORE, Harness, PLUGIN, TOKEN, WHOAMI};
use crate::members::MemberRecord;
use crate::plugin::PluginPackage;
use crate::plugin::languages::Languages;
use crate::plugin::storefront::index;

#[tokio::test]
async fn the_family_switch_gates_the_list_the_webui_and_the_detail() {
    let h = Harness::start(None).await;
    let hers = SecretKey::generate().public();
    invite_record(&h, false).await;
    h.tunnel
        .devices
        .add_if(hers, None, Some("mia".to_owned()), |_| true)
        .unwrap();

    // 开关默认关：列表里没有，详情和网页都进不去。
    assert_gated(&h, hers, Some("not_shared")).await;

    // 家人的开关测试需要插件记录（register_for_test 只填内存表）。
    write_whoami_record(&h);

    // 主人打开开关：列表、详情、网页都放行，网页的请求转发到插件。
    let reply = send_as(
        &h.app,
        None,
        Method::PUT,
        CORE,
        "/_wonton/api/plugins/app.whoami/family",
        &[
            ("authorization", bearer().as_str()),
            ("content-type", "application/json"),
        ],
        "{\"enabled\": true}",
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    assert_eq!(reply.json()["family"], true);

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
    assert_eq!(
        reply.json()["plugins"].as_array().unwrap().len(),
        1,
        "{}",
        reply.text()
    );
    assert_opened(&h, hers).await;
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

/// 开关关着：列表为空，详情和网页都回 403，错误码是 `kind`。
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

/// 家人装插件的开关：关着回 403 install_not_allowed，打开后能走到和主人一样的路。
#[tokio::test]
async fn installing_plugins_needs_the_owners_grant() {
    let h = Harness::start(None).await;
    let hers = SecretKey::generate().public();
    invite_record(&h, false).await;
    h.tunnel
        .devices
        .add_if(hers, None, Some("mia".to_owned()), |_| true)
        .unwrap();
    // 装上 echo（FakeDocker 建容器、写记录）：授权前的 403 门在 install 之前。
    let compose = WHOAMI
        .replace("app.whoami", "app.echo")
        .replace("  icon: icon.svg\n", "")
        .replace("  webui:\n    service: web\n    port: 8000\n", "");
    h.manager
        .install(&compose, &[], Languages::from_header(None))
        .await
        .unwrap();

    // 没授权的家人调装卸升级类：403 install_not_allowed。
    for (method, path, body) in [
        (
            Method::POST,
            "/_wonton/api/plugins/preview",
            "{\"source\": \"github:o/r\"}",
        ),
        (Method::POST, "/_wonton/api/plugins/app.echo/start", ""),
        (Method::DELETE, "/_wonton/api/plugins/app.echo", ""),
        (
            Method::POST,
            "/_wonton/api/plugins/app.echo/upgrade/preview",
            "",
        ),
    ] {
        let reply = send_as(&h.app, Some(hers), method.clone(), CORE, path, &[], body).await;
        assert_eq!(
            reply.status,
            StatusCode::FORBIDDEN,
            "{method} {path}: {}",
            reply.text()
        );
        assert_eq!(
            reply.error_kind(),
            Some("install_not_allowed"),
            "{method} {path}: {}",
            reply.text()
        );
    }

    // 主人授权后能走管理接口，也看得到全部插件。
    let reply = send_as(
        &h.app,
        None,
        Method::PUT,
        CORE,
        "/_wonton/api/members/mia",
        &[
            ("authorization", bearer().as_str()),
            ("content-type", "application/json"),
        ],
        "{\"can_install_plugins\": true}",
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());

    // 能装插件的家人看得到全部插件；启动放行。
    assert_listed_after_grant(&h, hers).await;
}

/// 记一个“mia”：`can_install_plugins` 按测试要的开。
async fn invite_record(h: &Harness, can_install: bool) {
    h.members
        .invite(MemberRecord {
            username: "mia".to_owned(),
            user_id: "@mia:test".to_owned(),
            display_name: "Mia".to_owned(),
            email: None,
            status: wonton_protocol::MemberStatus::Invited,
            can_install_plugins: can_install,
            wants_private_bot: true,
            private_bot: None,
            pairing_code_hash: None,
            pairing_code_expires_at_ms: None,
        })
        .unwrap();
}

/// 授权后：列表看得到装着的插件，启动放行。
async fn assert_listed_after_grant(h: &Harness, hers: iroh::EndpointId) {
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
    assert!(
        reply.json()["plugins"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["id"] == "app.echo"),
        "{}",
        reply.text()
    );
    let reply = send_as(
        &h.app,
        Some(hers),
        Method::POST,
        CORE,
        "/_wonton/api/plugins/app.echo/start",
        &[],
        "",
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
}

/// 授权了“能装插件”的家人看商店和主人一样；没授权的 403（路由表）。
#[tokio::test]
async fn a_family_member_with_install_grant_reads_the_store() {
    // 本地索引：不用外网就有商店内容。
    let index = tempfile::tempdir().unwrap();
    let compose = WHOAMI
        .replace("  icon: icon.svg\n", "")
        .replace("  webui:\n    service: web\n    port: 8000\n", "");
    let package = PluginPackage::parse(&compose, &Languages::from_header(None)).unwrap();
    index::IndexEntry::of(&package, "github:example/whoami", &"a".repeat(40))
        .unwrap()
        .write_to(index.path(), &package)
        .unwrap();
    let h = Harness::start_with_store(
        None,
        crate::plugin::storefront::StoreConfig {
            index_dir: Some(index.path().to_owned()),
            community: false,
        },
    )
    .await;
    let hers = SecretKey::generate().public();
    h.tunnel
        .devices
        .add_if(hers, None, Some("mia".to_owned()), |_| true)
        .unwrap();
    invite_record(&h, false).await;

    // 没授权：403 install_not_allowed（路由表里也过，这里看有索引时的行为）。
    let reply = send_as(
        &h.app,
        Some(hers),
        Method::GET,
        CORE,
        "/_wonton/api/store",
        &[],
        "",
    )
    .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN, "{}", reply.text());
    assert_eq!(reply.error_kind(), Some("install_not_allowed"));

    // 主人授权后：商店读得到，官方段里有索引里的插件。
    let reply = send_as(
        &h.app,
        None,
        Method::PUT,
        CORE,
        "/_wonton/api/members/mia",
        &[
            ("authorization", bearer().as_str()),
            ("content-type", "application/json"),
        ],
        "{\"can_install_plugins\": true}",
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    let reply = send_as(
        &h.app,
        Some(hers),
        Method::GET,
        CORE,
        "/_wonton/api/store",
        &[],
        "",
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    assert!(
        !reply.json()["official"]["plugins"]
            .as_array()
            .unwrap()
            .is_empty(),
        "{}",
        reply.text()
    );
}

/// 卡片都发给主人：家人列出来是空的，回答别人的卡片回 403 `not_yours`。
#[tokio::test]
async fn confirmations_still_belong_to_the_owner() {
    let h = Harness::start(None).await;
    let hers = SecretKey::generate().public();
    invite_record(&h, false).await;
    h.tunnel
        .devices
        .add_if(hers, None, Some("mia".to_owned()), |_| true)
        .unwrap();

    let reply = send_as(
        &h.app,
        Some(hers),
        Method::GET,
        CORE,
        "/_wonton/api/confirmations",
        &[],
        "",
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    assert_eq!(reply.json()["confirmations"].as_array().unwrap().len(), 0);

    for (method, path, body) in [
        (
            Method::POST,
            "/_wonton/confirmations/some-id",
            "{\"answer\": \"approve\"}",
        ),
        (Method::DELETE, "/_wonton/confirmations/some-id/window", ""),
    ] {
        let reply = send_as(
            &h.app,
            Some(hers),
            method,
            CORE,
            path,
            &[("content-type", "application/json")],
            body,
        )
        .await;
        assert_eq!(reply.status, StatusCode::FORBIDDEN, "{}", reply.text());
        assert_eq!(reply.error_kind(), Some("not_yours"), "{}", reply.text());
    }
}

/// 网页的身份给当前这个人：家人的 OIDC 声明是她的用户名和 member 角色，主人的照旧。
#[tokio::test]
async fn oidc_claims_come_from_the_current_person() {
    let accepted = Arc::new(Mutex::new(Vec::new()));
    let hydra = fake_hydra_for(accepted.clone(), "mia").await;
    let h = Harness::start(Some(hydra)).await;
    let hers = SecretKey::generate().public();
    invite_record(&h, false).await;
    h.tunnel
        .devices
        .add_if(hers, None, Some("mia".to_owned()), |_| true)
        .unwrap();

    let reply = send_as(
        &h.app,
        Some(hers),
        Method::GET,
        CORE,
        "/_wonton/oidc/consent?consent_challenge=def",
        &[],
        "",
    )
    .await;
    assert_eq!(reply.status, StatusCode::SEE_OTHER, "{}", reply.text());
    let claims = claims_of(&accepted, 0);
    assert_eq!(claims["preferred_username"], "mia");
    assert_eq!(claims["groups"], json!(["member"]));

    // 主人的声明照旧（主人令牌登录的核心会话）。假 Hydra 对这个 challenge 返回主人的 subject。
    let core = h.login(TOKEN).await.cookie(super::CORE_COOKIE);
    let reply = send_as(
        &h.app,
        None,
        Method::GET,
        CORE,
        "/_wonton/oidc/consent?consent_challenge=ghi",
        &[("cookie", core.as_str())],
        "",
    )
    .await;
    assert_eq!(reply.status, StatusCode::SEE_OTHER, "{}", reply.text());
    let claims = claims_of(&accepted, 1);
    assert_eq!(claims["preferred_username"], "owner");
    assert_eq!(claims["groups"], json!(["owner"]));
}

fn claims_of(accepted: &Mutex<Vec<Value>>, nth: usize) -> Value {
    accepted.lock().unwrap()[nth]["session"]["id_token"].clone()
}
