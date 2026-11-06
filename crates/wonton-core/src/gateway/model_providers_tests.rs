use std::fs;

use axum::body::Body;
use hyper::{Method, StatusCode};
use serde_json::json;

use super::tests::{CORE, Harness};
use crate::plugin::models::ModelsRoute;
use crate::plugin::names::container_name;
use crate::plugin::storefront::StoreConfig;
use crate::plugin::{PluginPackage, PluginStatus};

const PROVIDER: &str = "services:\n  magpie:\n    image: magpie\nx-wonton:\n  spec: '0.6'\n  id: system.models\n  version: 0.6.0\n  kind: system\n  title: {zh: 模型, en: Models}\n  description: {zh: 模型, en: Models}\n  webui: {service: magpie, port: 3430}\n  provides:\n    models:\n      service: magpie\n      port: 3425\n      apis: {openai: /v1}\n      default_model: group/default\n      credentials:\n        issue: [issue, '{name}']\n        revoke: [revoke, '{name}']\n      providers:\n        list: [providers, list]\n        save: [providers, save, '{id}']\n        test: [providers, test, '{id}']\n        custom:\n          save: [providers, custom-save]\n          test: [providers, custom-test]\n  permissions: []";

const PROVIDER_WITHOUT_COMMANDS: &str = "services:\n  magpie:\n    image: magpie\nx-wonton:\n  spec: '0.6'\n  id: system.models\n  version: 0.6.0\n  kind: system\n  title: {zh: 模型, en: Models}\n  description: {zh: 模型, en: Models}\n  webui: {service: magpie, port: 3430}\n  provides:\n    models:\n      service: magpie\n      port: 3425\n      apis: {openai: /v1}\n      default_model: group/default\n      credentials:\n        issue: [issue, '{name}']\n        revoke: [revoke, '{name}']\n  permissions: []";

fn command(cmd: &[String], expected: &[&str]) -> bool {
    cmd.iter().map(String::as_str).eq(expected.iter().copied())
}

async fn running_gateway() -> Harness {
    gateway(Harness::start(None).await)
}

fn gateway(harness: Harness) -> Harness {
    harness
        .docker
        .add_container(&container_name("system-models", "magpie"), "system.models");
    let manifest = PluginPackage::parse(PROVIDER).unwrap().manifest;
    let route = ModelsRoute {
        id: manifest.id.clone(),
        spec: manifest.provides.clone().unwrap().models.unwrap(),
        upstream: "127.0.0.1:3425".into(),
        credentials_container: container_name("system-models", "magpie"),
    };
    harness
        .manager
        .register_for_test(manifest, PluginStatus::Running, None);
    harness.manager.serve_models_for_test(route);
    harness.docker.on_exec_output(|_, cmd, _| {
        if command(cmd, &["providers", "list"]) {
            return (
                0,
                r#"{"presets":[{"id":"deepseek","title":"DeepSeek"},{"id":"openrouter","title":"OpenRouter"}],"configured":["openrouter"]}"#.into(),
                String::new(),
            );
        }
        (0, String::new(), String::new())
    });
    harness
}

fn auth_header() -> (&'static str, &'static str) {
    ("authorization", "Bearer test-owner-token")
}

#[tokio::test]
async fn provider_api_uses_plugin_authentication() {
    let harness = running_gateway().await;
    let path = wonton_protocol::MODEL_PROVIDERS_PATH;
    let denied = harness.get(CORE, path, &[]).await;
    assert_eq!(denied.status, StatusCode::UNAUTHORIZED);
    let allowed = harness.get(CORE, path, &[auth_header()]).await;
    assert_eq!(allowed.status, StatusCode::OK, "{}", allowed.text());
}

fn catalog() -> serde_json::Value {
    json!({
        "agents": [], "needs": [], "prompts": [],
        "providers": {
            "en": [
                {"id": "openrouter", "key_url": "https://catalog.example/openrouter"},
                {"id": "deepseek", "key_url": "https://catalog.example/deepseek"},
                {"id": "openrouter", "key_url": "https://duplicate.example/recommendation"},
                {"id": "missing-recommended", "key_url": "https://catalog.example/missing"}
            ],
            "zh": [{"id": "other-language", "key_url": "https://catalog.example/other"}]
        },
        "more_providers": [
            {"id": "gemini", "key_url": "https://catalog.example/gemini"},
            {"id": "openrouter", "key_url": "https://duplicate.example/overlap"},
            {"id": "anthropic", "key_url": "https://catalog.example/anthropic"},
            {"id": "gemini", "key_url": "https://duplicate.example/more"},
            {"id": "missing-more", "key_url": "https://catalog.example/missing-more"}
        ]
    })
}

async fn gateway_with_index(index: Option<serde_json::Value>) -> (Harness, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    if let Some(index) = index {
        fs::write(
            dir.path().join("index.json"),
            serde_json::to_vec(&index).unwrap(),
        )
        .unwrap();
    }
    let harness = gateway(
        Harness::start_with_store(
            None,
            StoreConfig {
                index_dir: Some(dir.path().to_owned()),
                community: false,
            },
        )
        .await,
    );
    (harness, dir)
}

#[tokio::test]
async fn list_filters_and_orders_catalog_providers() {
    let (harness, _index) = gateway_with_index(Some(json!({
        "format": 1, "plugins": [], "onboarding": catalog()
    })))
    .await;
    harness.docker.on_exec_output(|_, _, _| {
        (
            0,
            json!({
                "presets": [
                    {"id": "apikeyfan", "title": "APIKeyFan"},
                    {"id": "anthropic", "title": "Anthropic"},
                    {"id": "deepseek", "title": "DeepSeek"},
                    {"id": "gemini", "title": "Gemini"},
                    {"id": "openrouter", "title": "OpenRouter"},
                    {"id": "other-language", "title": "Other"},
                    {"id": "gemini", "title": "Repeated Gemini"},
                    {"id": "openrouter", "title": "Repeated OpenRouter"}
                ],
                "configured": ["apikeyfan"]
            })
            .to_string(),
            String::new(),
        )
    });
    let response = harness
        .get(
            CORE,
            wonton_protocol::MODEL_PROVIDERS_PATH,
            &[auth_header(), ("accept-language", "en")],
        )
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    let body = response.json();
    let providers = body["providers"].as_array().unwrap();
    assert_eq!(providers.len(), 4);
    for (provider, (id, recommended)) in providers.iter().zip([
        ("openrouter", true),
        ("deepseek", true),
        ("gemini", false),
        ("anthropic", false),
    ]) {
        assert_eq!(provider["id"], id);
        assert_eq!(provider["recommended"], recommended);
        assert_eq!(provider["configured"], false);
        assert_eq!(provider["key_url"], format!("https://catalog.example/{id}"));
    }
    assert_eq!(body["any_configured"], true);
    assert_eq!(body["custom"], true);
    assert!(body["web_url"].as_str().unwrap().contains("system-models"));
}

#[tokio::test]
async fn missing_index_or_catalog_returns_empty_providers() {
    for index in [None, Some(json!({"format": 1, "plugins": []}))] {
        let (harness, _index) = gateway_with_index(index).await;
        let response = harness
            .get(
                CORE,
                wonton_protocol::MODEL_PROVIDERS_PATH,
                &[auth_header()],
            )
            .await;
        assert_eq!(response.status, StatusCode::OK, "{}", response.text());
        let body = response.json();
        assert_eq!(body["providers"], json!([]));
        assert_eq!(body["any_configured"], true);
        assert!(body["web_url"].as_str().unwrap().contains("system-models"));
    }
}

#[tokio::test]
async fn list_selects_requested_language_and_preserves_configuration() {
    for configured in [false, true] {
        let (harness, _index) = gateway_with_index(Some(json!({
            "format": 1, "plugins": [], "onboarding": catalog()
        })))
        .await;
        harness.docker.on_exec_output(move |_, _, _| {
            (
                0,
                json!({
                    "presets": [
                        {"id": "deepseek", "title": "DeepSeek"},
                        {"id": "other-language", "title": "Other"}
                    ],
                    "configured": if configured { vec!["other-language"] } else { vec![] }
                })
                .to_string(),
                String::new(),
            )
        });
        let response = harness
            .get(
                CORE,
                wonton_protocol::MODEL_PROVIDERS_PATH,
                &[auth_header(), ("accept-language", "zh")],
            )
            .await;
        assert_eq!(response.status, StatusCode::OK, "{}", response.text());
        let body = response.json();
        assert_eq!(body["providers"].as_array().unwrap().len(), 1);
        assert_eq!(body["providers"][0]["id"], "other-language");
        assert_eq!(body["providers"][0]["recommended"], true);
        assert_eq!(body["providers"][0]["configured"], configured);
        assert_eq!(
            body["providers"][0]["key_url"],
            "https://catalog.example/other"
        );
        assert_eq!(body["any_configured"], configured);
    }
}

#[tokio::test]
async fn malformed_or_failed_list_does_not_expose_gateway_output() {
    for (code, stdout) in [(0, r#"{"presets":"secret-key"}"#), (1, "secret-key")] {
        let harness = running_gateway().await;
        harness
            .docker
            .on_exec_output(move |_, _, _| (code, stdout.into(), "secret-key from stderr".into()));
        let response = harness
            .get(
                CORE,
                wonton_protocol::MODEL_PROVIDERS_PATH,
                &[auth_header()],
            )
            .await;
        assert_eq!(response.status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(response.error_kind(), Some("models_unavailable"));
        assert!(!response.text().contains("secret-key"));
    }
}

#[tokio::test]
async fn gateway_not_running_is_models_unavailable() {
    let harness = Harness::start(None).await;
    let manifest = PluginPackage::parse(PROVIDER).unwrap().manifest;
    harness.manager.register_for_test(
        manifest,
        PluginStatus::Degraded {
            reason: "down".into(),
        },
        None,
    );
    let response = harness
        .get(
            CORE,
            wonton_protocol::MODEL_PROVIDERS_PATH,
            &[auth_header()],
        )
        .await;
    assert_eq!(response.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(response.error_kind(), Some("models_unavailable"));
}

#[tokio::test]
async fn gateway_without_provider_commands_returns_web_url() {
    let harness = Harness::start(None).await;
    let manifest = PluginPackage::parse(PROVIDER_WITHOUT_COMMANDS)
        .unwrap()
        .manifest;
    let route = ModelsRoute {
        id: manifest.id.clone(),
        spec: manifest.provides.clone().unwrap().models.unwrap(),
        upstream: "127.0.0.1:3425".into(),
        credentials_container: container_name("system-models", "magpie"),
    };
    harness
        .manager
        .register_for_test(manifest, PluginStatus::Running, None);
    harness.manager.serve_models_for_test(route);
    let response = harness
        .get(
            CORE,
            wonton_protocol::MODEL_PROVIDERS_PATH,
            &[auth_header()],
        )
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    let body = response.json();
    assert!(body["providers"].is_null());
    assert!(body["web_url"].as_str().unwrap().contains("system-models"));
    assert_eq!(body["custom"], false);
}

fn operations() -> [(Method, String, serde_json::Value); 4] {
    let key = json!({"key": "secret-key"});
    let custom =
        json!({"base_url": "http://192.0.2.10/v1", "model": "exact-model", "key": "secret-key"});
    [
        (
            Method::PUT,
            wonton_protocol::model_provider_path("deepseek"),
            key.clone(),
        ),
        (
            Method::POST,
            wonton_protocol::model_provider_test_path("deepseek"),
            key,
        ),
        (
            Method::PUT,
            wonton_protocol::CUSTOM_MODEL_PROVIDER_PATH.into(),
            custom.clone(),
        ),
        (
            Method::POST,
            wonton_protocol::CUSTOM_MODEL_PROVIDER_TEST_PATH.into(),
            custom,
        ),
    ]
}

#[tokio::test]
async fn successful_provider_commands_return_no_content() {
    let harness = running_gateway().await;
    for (method, path, body) in operations() {
        let response = harness
            .send(
                method,
                CORE,
                &path,
                &[auth_header()],
                Body::from(body.to_string()),
            )
            .await;
        assert_eq!(response.status, StatusCode::NO_CONTENT);
        assert!(response.text().is_empty());
    }
}

#[tokio::test]
async fn provider_exit_codes_map_without_returning_command_output() {
    for (method, path, body) in operations() {
        for (code, status, kind) in [
            (2, StatusCode::BAD_REQUEST, "invalid_input"),
            (3, StatusCode::BAD_REQUEST, "invalid_input"),
            (4, StatusCode::BAD_GATEWAY, "upstream_unavailable"),
            (9, StatusCode::SERVICE_UNAVAILABLE, "models_unavailable"),
        ] {
            let harness = running_gateway().await;
            harness.docker.on_exec_output(move |_, cmd, _| {
                if command(cmd, &["providers", "list"]) {
                    return (
                        0,
                        r#"{"presets":[{"id":"deepseek","title":"DeepSeek"}],"configured":[]}"#
                            .into(),
                        String::new(),
                    );
                }
                (
                    code,
                    "secret-key from stdout".into(),
                    "secret-key from stderr".into(),
                )
            });
            let response = harness
                .send(
                    method.clone(),
                    CORE,
                    &path,
                    &[auth_header()],
                    Body::from(body.to_string()),
                )
                .await;
            let (status, kind) = if code == 2 && path.contains("/providers/") {
                (StatusCode::SERVICE_UNAVAILABLE, "models_unavailable")
            } else {
                (status, kind)
            };
            assert_eq!(response.status, status, "{}", response.text());
            assert_eq!(response.error_kind(), Some(kind));
            assert!(!response.text().contains("secret-key"));
        }
    }
}

#[tokio::test]
async fn provider_writes_require_owner_and_same_origin_and_limit_body() {
    let harness = running_gateway().await;
    let origin = harness.origin("app-whoami.wonton.localhost");
    for (method, path, body) in operations() {
        let denied = harness
            .send(
                method.clone(),
                CORE,
                &path,
                &[],
                Body::from(body.to_string()),
            )
            .await;
        assert_eq!(denied.status, StatusCode::UNAUTHORIZED);
        let cross_site = harness
            .send(
                method.clone(),
                CORE,
                &path,
                &[auth_header(), ("origin", &origin)],
                Body::from(body.to_string()),
            )
            .await;
        assert_eq!(cross_site.status, StatusCode::FORBIDDEN);
        let oversized = harness
            .send(
                method.clone(),
                CORE,
                &path,
                &[auth_header()],
                Body::from(" ".repeat((8 << 10) + 1)),
            )
            .await;
        assert_eq!(oversized.status, StatusCode::PAYLOAD_TOO_LARGE);
        let malformed = harness
            .send(
                method,
                CORE,
                &path,
                &[auth_header()],
                Body::from(r#""secret-key""#),
            )
            .await;
        assert_eq!(malformed.error_kind(), Some("invalid_input"));
        assert!(!malformed.text().contains("secret-key"));
    }
    assert!(harness.docker.execs().is_empty());
}

#[tokio::test]
async fn custom_without_capability_is_unavailable_and_does_not_execute() {
    let harness = running_gateway().await;
    let mut manifest = PluginPackage::parse(PROVIDER).unwrap().manifest;
    manifest
        .provides
        .as_mut()
        .unwrap()
        .models
        .as_mut()
        .unwrap()
        .providers
        .as_mut()
        .unwrap()
        .custom = None;
    harness.manager.serve_models_for_test(ModelsRoute {
        id: manifest.id,
        spec: manifest.provides.unwrap().models.unwrap(),
        upstream: "127.0.0.1:3425".into(),
        credentials_container: container_name("system-models", "magpie"),
    });
    for (method, path, body) in operations().into_iter().skip(2) {
        let response = harness
            .send(
                method,
                CORE,
                &path,
                &[auth_header()],
                Body::from(body.to_string()),
            )
            .await;
        assert_eq!(response.status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(response.error_kind(), Some("models_unavailable"));
    }
    assert!(harness.docker.execs().is_empty());
    let listed = harness
        .get(
            CORE,
            wonton_protocol::MODEL_PROVIDERS_PATH,
            &[auth_header()],
        )
        .await;
    assert_eq!(listed.json()["custom"], false);
}

#[tokio::test]
async fn invalid_custom_http_input_is_rejected_without_execution() {
    let harness = running_gateway().await;
    for (method, path, mut body) in operations().into_iter().skip(2) {
        body["base_url"] = json!("ftp://example.com");
        let response = harness
            .send(
                method,
                CORE,
                &path,
                &[auth_header()],
                Body::from(body.to_string()),
            )
            .await;
        assert_eq!(response.status, StatusCode::BAD_REQUEST);
        assert_eq!(response.error_kind(), Some("invalid_input"));
    }
    assert!(harness.docker.execs().is_empty());
}
