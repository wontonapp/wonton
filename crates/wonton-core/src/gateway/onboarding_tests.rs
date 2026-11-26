use std::fs;
use std::path::Path;

use axum::body::Body;
use hyper::{Method, StatusCode};
use serde_json::{Value, json};
use wonton_protocol::ONBOARDING_PATH;

use super::family_plugin_tests::app_with_contact_facts;
use super::family_tests::send_as;
use super::tests::{CORE, Harness};
use crate::members::MemberRecord;
use crate::plugin::languages::Languages;
use crate::plugin::storefront::StoreConfig;
use crate::plugin::storefront::index::IndexEntry;
use crate::plugin::{PluginPackage, PluginStatus};

const AGENT_COMPOSE: &str = r#"services:
  agent:
    image: nginx
x-wonton:
  spec: "0.6"
  id: com.example.agent
  version: 1.0.0
  kind: agent
  title: {zh: 示例 Agent, en: Example Agent}
  description: {zh: 示例 Agent, en: Example agent}
  agent: {protocol: matrix}
  permissions: [matrix.bot]
"#;

const PHOTOS_COMPOSE: &str = r#"services:
  photos:
    image: nginx
x-wonton:
  spec: "0.6"
  id: app.example.photos
  version: 1.0.0
  kind: app
  title: {zh: 照片, en: Photos}
  description: {zh: 备份照片, en: Back up photos}
  permissions: []
"#;
const EXTRA_AGENT_COMPOSE: &str = r#"services:
  agent:
    image: nginx
x-wonton:
  spec: "0.6"
  id: com.example.extra
  version: 1.0.0
  kind: agent
  title: {zh: 另一个 Agent, en: Another Agent}
  description: {zh: 另一个 Agent, en: Another agent}
  agent: {protocol: matrix}
  permissions: [matrix.bot]
"#;

fn write_catalog(dir: &Path) -> PluginPackage {
    let agent =
        PluginPackage::parse(AGENT_COMPOSE, &Languages::from_header(Some("zh-Hans"))).unwrap();
    let extra = PluginPackage::parse(
        EXTRA_AGENT_COMPOSE,
        &Languages::from_header(Some("zh-Hans")),
    )
    .unwrap();
    let photos =
        PluginPackage::parse(PHOTOS_COMPOSE, &Languages::from_header(Some("zh-Hans"))).unwrap();
    IndexEntry::of(&agent, "github:example/agent", &"a".repeat(40))
        .unwrap()
        .write_to(dir, &agent)
        .unwrap();
    IndexEntry::of(&photos, "github:example/photos", &"b".repeat(40))
        .unwrap()
        .write_to(dir, &photos)
        .unwrap();

    let path = dir.join("index.json");
    let mut index: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    // 商店删掉 `agents` 键之前它还在索引里：核心不读它，有也不影响。
    index["onboarding"] = json!({
        "agents": [extra.manifest.id.as_str()],
        "needs": [{
            "id": "photos",
            "title": {"zh": "备份照片", "en": "Back up photos"},
            "plugins": ["app.example.photos"]
        }],
        "prompts": [{"zh": "你能帮我做什么", "en": "What can you help me with"}]
    });
    fs::write(path, serde_json::to_vec_pretty(&index).unwrap()).unwrap();
    agent
}

fn auth<'a>() -> [(&'a str, &'a str); 2] {
    [
        ("authorization", "Bearer test-owner-token"),
        ("accept-language", "en"),
    ]
}

#[tokio::test]
async fn onboarding_get_put_persists_and_validates_requests() {
    let index = tempfile::tempdir().unwrap();
    let agent = write_catalog(index.path());
    let h = Harness::start_with_store(
        None,
        StoreConfig {
            index_dir: Some(index.path().to_owned()),
            community: false,
        },
    )
    .await;
    h.manager
        .register_for_test(agent.manifest.clone(), PluginStatus::Running, None);
    h.manager
        .set_default_bot_plugin(Some(agent.manifest.id.clone()));
    h.manager
        .bot_for_test(&agent.manifest.id, "@agent:example.com");

    assert_eq!(
        h.get(CORE, ONBOARDING_PATH, &[]).await.status,
        StatusCode::UNAUTHORIZED
    );
    let initial = h.get(CORE, ONBOARDING_PATH, &auth()).await;
    assert_eq!(initial.status, StatusCode::OK, "{}", initial.text());
    let initial = initial.json();
    assert_eq!(initial["step"], "profile");
    assert_eq!(initial["default_bot_plugin"]["id"], "com.example.agent");
    assert!(
        initial["default_bot_plugin"]["bot_user_id"].is_null(),
        "没接 Matrix 时调用者身份不可核实，Bot 身份不投影"
    );
    assert_eq!(
        initial["needs"][0]["plugins"][0]["id"],
        "app.example.photos"
    );
    assert!(initial["catalog_problem"].is_null());

    let origin = h.origin("app-whoami.wonton.localhost");
    let cross_site = [
        ("authorization", "Bearer test-owner-token"),
        ("origin", origin.as_str()),
    ];
    let rejected = h
        .send(
            Method::PUT,
            CORE,
            ONBOARDING_PATH,
            &cross_site,
            Body::from(r#"{"step":"finished"}"#),
        )
        .await;
    assert_eq!(rejected.status, StatusCode::FORBIDDEN);
    assert_eq!(rejected.error_kind(), Some("cross_site"));

    let mut headers = auth();
    headers[1] = ("content-type", "application/json");
    let updated = h
        .send(
            Method::PUT,
            CORE,
            ONBOARDING_PATH,
            &headers,
            Body::from(r#"{"step":"finished"}"#),
        )
        .await;
    assert_eq!(updated.status, StatusCode::OK, "{}", updated.text());
    let updated = updated.json();
    assert_eq!(updated["step"], "finished");
    assert_eq!(updated["default_bot_plugin"]["id"], "com.example.agent");

    let restarted = h
        .send(
            Method::PUT,
            CORE,
            ONBOARDING_PATH,
            &headers,
            Body::from(r#"{"step":"profile"}"#),
        )
        .await
        .json();
    assert_eq!(restarted["step"], "profile");

    assert_invalid_updates(&h, &headers).await;
}

#[tokio::test]
async fn the_default_bot_plugin_comes_from_the_default_install_not_the_catalog() {
    let index = tempfile::tempdir().unwrap();
    let agent = write_catalog(index.path());
    let h = Harness::start_with_store(
        None,
        StoreConfig {
            index_dir: Some(index.path().to_owned()),
            community: false,
        },
    )
    .await;
    let extra = PluginPackage::parse(
        EXTRA_AGENT_COMPOSE,
        &Languages::from_header(Some("zh-Hans")),
    )
    .unwrap();
    // 商店推荐的、已经装着的另一个 Agent 都不算默认：默认只看默认安装。
    h.manager
        .register_for_test(extra.manifest.clone(), PluginStatus::Running, None);
    let view = h.get(CORE, ONBOARDING_PATH, &auth()).await.json();
    assert!(view["default_bot_plugin"].is_null(), "{view}");
    assert!(view.get("agents").is_none(), "{view}");

    // 主人卸掉了默认安装的 Agent：仍是它，如实显示它的 Bot 不在。
    h.manager
        .set_default_bot_plugin(Some(agent.manifest.id.clone()));
    let view = h.get(CORE, ONBOARDING_PATH, &auth()).await.json();
    assert_eq!(view["default_bot_plugin"]["id"], "com.example.agent");
    assert_eq!(view["default_bot_plugin"]["visibility"], "absent");
}

#[tokio::test]
async fn onboarding_marks_other_peoples_private_bot_absent_for_a_family_admin() {
    const OWNER_BOT: &str = "@owner-agent:test";
    let index = tempfile::tempdir().unwrap();
    let agent = write_catalog(index.path());
    let h = Harness::start_with_store(
        None,
        StoreConfig {
            index_dir: Some(index.path().to_owned()),
            community: false,
        },
    )
    .await;
    let (app, _matrix_state) = app_with_contact_facts(&h).await;
    h.manager
        .register_for_test(agent.manifest.clone(), PluginStatus::Running, None);
    h.manager
        .set_default_bot_plugin(Some(agent.manifest.id.clone()));
    h.manager.bot_for_test(&agent.manifest.id, OWNER_BOT);
    h.members
        .invite(MemberRecord {
            role_id: "admin".into(),
            wants_private_bot: false,
            ..MemberRecord::for_test("mia")
        })
        .unwrap();
    h.members.mark_joined("mia").unwrap();
    let owner = iroh::SecretKey::generate().public();
    h.tunnel.devices.add(owner, None).unwrap();
    let family = iroh::SecretKey::generate().public();
    h.tunnel
        .devices
        .add_if(family, None, Some("mia".into()), |_| true)
        .unwrap();

    for method in [Method::GET, Method::PUT] {
        for (device, allowed) in [(owner, true), (family, false)] {
            let reply = send_as(
                &app,
                Some(device),
                method.clone(),
                CORE,
                ONBOARDING_PATH,
                &[("content-type", "application/json")],
                r#"{"step":"finished"}"#,
            )
            .await;
            assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
            assert_eq!(
                reply
                    .json()
                    .pointer("/default_bot_plugin/bot_user_id")
                    .and_then(Value::as_str),
                Some(OWNER_BOT),
            );
            assert_eq!(
                reply
                    .json()
                    .pointer("/default_bot_plugin/visibility")
                    .and_then(Value::as_str),
                Some(if allowed { "visible" } else { "absent" }),
            );
        }
    }
}

async fn assert_invalid_updates(h: &Harness, headers: &[(&str, &str); 2]) {
    for body in [r#"{"step":"other"}"#, r#"{"step":"apps"}"#, r#"not-json"#] {
        let reply = h
            .send(
                Method::PUT,
                CORE,
                ONBOARDING_PATH,
                headers,
                Body::from(body),
            )
            .await;
        assert_eq!(
            reply.status,
            StatusCode::BAD_REQUEST,
            "{body}: {}",
            reply.text()
        );
        assert_eq!(reply.error_kind(), Some("invalid_input"));
    }
}

#[tokio::test]
async fn onboarding_keeps_state_when_the_catalog_becomes_unavailable() {
    let index = tempfile::tempdir().unwrap();
    let agent = write_catalog(index.path());
    let h = Harness::start_with_store(
        None,
        StoreConfig {
            index_dir: Some(index.path().to_owned()),
            community: false,
        },
    )
    .await;
    h.manager
        .register_for_test(agent.manifest.clone(), PluginStatus::Running, None);
    h.manager
        .set_default_bot_plugin(Some(agent.manifest.id.clone()));
    let headers = [
        ("authorization", "Bearer test-owner-token"),
        ("content-type", "application/json"),
    ];
    let update = h
        .send(
            Method::PUT,
            CORE,
            ONBOARDING_PATH,
            &headers,
            Body::from(r#"{"step":"finished"}"#),
        )
        .await;
    assert_eq!(update.status, StatusCode::OK, "{}", update.text());

    fs::remove_file(index.path().join("index.json")).unwrap();
    let response = h
        .get(
            CORE,
            ONBOARDING_PATH,
            &[("authorization", "Bearer test-owner-token")],
        )
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    let view = response.json();
    assert_eq!(view["step"], "finished");
    assert_eq!(view["default_bot_plugin"]["id"], "com.example.agent");
    assert_eq!(view["catalog_problem"]["reason"], "upstream");
    assert_eq!(view["needs"], json!([]));
    assert_eq!(view["prompts"], json!([]));
}

#[tokio::test]
async fn onboarding_state_survives_gateway_rebuild_and_corruption_is_reported() {
    let h = Harness::start(None).await;
    let path = "/_wonton/api/onboarding";
    let auth = [
        ("authorization", "Bearer test-owner-token"),
        ("content-type", "application/json"),
    ];
    let updated = h
        .send(
            Method::PUT,
            CORE,
            path,
            &auth,
            Body::from(r#"{"step":"finished"}"#),
        )
        .await;
    assert_eq!(updated.status, StatusCode::OK, "{}", updated.text());

    let state_path = h.data_dir().join("core/onboarding.json");
    let response = h.get_after_restart(CORE, path, &auth[..1]).await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    assert_eq!(response.json()["step"], "finished");

    fs::write(&state_path, b"not json").unwrap();
    let response = h.get_after_restart(CORE, path, &auth[..1]).await;
    assert_eq!(response.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(response.error_kind(), Some("storage"));
    assert_eq!(fs::read(&state_path).unwrap(), b"not json");
}
