use std::fs;
use std::path::Path;

use axum::body::Body;
use hyper::{Method, StatusCode};
use serde_json::{Value, json};
use wonton_protocol::ONBOARDING_PATH;

use super::tests::{CORE, Harness};
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
  agent:
    protocol: acp
    login:
      - id: chatgpt
        title: {zh: ChatGPT 账号, en: ChatGPT account}
  permissions: [network.egress]
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
  agent: {protocol: acp}
  permissions: []
"#;

fn write_catalog(dir: &Path) -> PluginPackage {
    let agent =
        PluginPackage::parse(AGENT_COMPOSE, &Languages::from_header(Some("zh-Hans"))).unwrap();
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
    index["onboarding"] = json!({
        "agents": ["com.example.agent"],
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
    let extra = PluginPackage::parse(
        EXTRA_AGENT_COMPOSE,
        &Languages::from_header(Some("zh-Hans")),
    )
    .unwrap();
    let agent_id = agent.manifest.id.clone();
    h.manager
        .register_for_test(agent.manifest.clone(), PluginStatus::Running, None);
    h.manager
        .register_for_test(extra.manifest.clone(), PluginStatus::Running, None);
    h.manager.bot_for_test(&agent_id, "@agent:example.com");

    assert_eq!(
        h.get(CORE, ONBOARDING_PATH, &[]).await.status,
        StatusCode::UNAUTHORIZED
    );
    let initial = h.get(CORE, ONBOARDING_PATH, &auth()).await;
    assert_eq!(initial.status, StatusCode::OK, "{}", initial.text());
    let initial = initial.json();
    assert_eq!(initial["agents"][0]["id"], "com.example.agent");
    assert_eq!(initial["agents"][1]["id"], "com.example.extra");
    assert_eq!(initial["agents"].as_array().unwrap().len(), 2);
    assert_eq!(initial["agents"][0]["logins"][0]["id"], "chatgpt");
    assert_eq!(initial["agents"][0]["bot_user_id"], "@agent:example.com");
    assert_eq!(initial["agents"][0]["status"]["state"], "running");
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
            Body::from(r#"{"step":"apps"}"#),
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
            Body::from(
                json!({
                    "step": "finished",
                    "primary_agent": "com.example.agent",
                    "replaced_agent_asked": true
                })
                .to_string(),
            ),
        )
        .await;
    assert_eq!(updated.status, StatusCode::OK, "{}", updated.text());
    let updated = updated.json();
    assert_eq!(updated["step"], "finished");
    assert_eq!(updated["primary_agent"], "com.example.agent");
    assert_eq!(updated["replaced_agent_asked"], true);

    let partial = h
        .send(
            Method::PUT,
            CORE,
            ONBOARDING_PATH,
            &headers,
            Body::from(r#"{"step":"profile"}"#),
        )
        .await
        .json();
    assert_eq!(partial["step"], "profile");
    assert_eq!(partial["primary_agent"], "com.example.agent");
    assert_eq!(partial["replaced_agent_asked"], true);

    assert_invalid_updates(&h, &headers).await;
}

async fn assert_invalid_updates(h: &Harness, headers: &[(&str, &str); 2]) {
    for body in [
        r#"{"step":"other"}"#,
        r#"{"step":"apps"}"#,
        r#"{"primary_agent":"com.example.missing"}"#,
        r#"not-json"#,
    ] {
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
            Body::from(r#"{"step":"finished","primary_agent":"com.example.agent"}"#),
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
    assert_eq!(view["primary_agent"], "com.example.agent");
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
