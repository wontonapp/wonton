use serde_json::json;

use super::*;

#[test]
fn command_input_is_one_flat_json_object_with_op() {
    let input = ModelsCommandInput {
        op: ModelsOp::SaveKey {
            provider: "openrouter".into(),
            key: SecretString::new("sk-or-secret"),
            base_url: None,
        },
        bot: "alice".into(),
        languages: vec!["zh-Hans".into(), "en".into()],
    };
    let value = serde_json::to_value(&input).unwrap();
    assert_eq!(
        value,
        json!({"op": "save_key", "provider": "openrouter", "key": "sk-or-secret",
               "bot": "alice", "languages": ["zh-Hans", "en"]})
    );
    assert_eq!(
        serde_json::from_value::<ModelsCommandInput>(value).unwrap(),
        input
    );
    assert!(!format!("{input:?}").contains("sk-or-secret"));
}

#[test]
fn every_op_has_a_snake_case_name() {
    let ops = [
        (ModelsOp::Catalog, "catalog"),
        (
            ModelsOp::Disconnect {
                provider: "x".into(),
            },
            "disconnect",
        ),
        (
            ModelsOp::LoginStart {
                provider: "x".into(),
                redirect_uri: Some(MODEL_LOGIN_REDIRECT_URI.into()),
            },
            "login_start",
        ),
        (
            ModelsOp::LoginPoll {
                session: "s".into(),
            },
            "login_poll",
        ),
        (
            ModelsOp::LoginFinish {
                session: "s".into(),
                callback_url: "wonton://model-login?code=1".into(),
            },
            "login_finish",
        ),
        (
            ModelsOp::LoginCancel {
                session: "s".into(),
            },
            "login_cancel",
        ),
        (
            ModelsOp::Models {
                provider: "x".into(),
            },
            "models",
        ),
        (
            ModelsOp::Select {
                provider: "x".into(),
                model: "m".into(),
                confirmed: false,
            },
            "select",
        ),
        (ModelsOp::Test, "test"),
    ];
    for (op, name) in ops {
        assert_eq!(serde_json::to_value(&op).unwrap()["op"], name);
    }
    let unconfirmed = ModelsOp::Select {
        provider: "x".into(),
        model: "m".into(),
        confirmed: false,
    };
    assert!(
        serde_json::to_value(unconfirmed)
            .unwrap()
            .get("confirmed")
            .is_none()
    );
}

#[test]
fn newer_values_from_the_plugin_fall_back_to_other() {
    let catalog: ModelCatalog = serde_json::from_value(json!({
        "providers": [{
            "id": "nous", "name": "Nous Portal", "recommended": true,
            "auth": [{"type": "device_login"}, {"type": "passkey", "rp": "example.com"}],
            "connected": false, "extra": 1
        }],
        "current": null
    }))
    .unwrap();
    assert_eq!(
        catalog.providers[0].auth,
        vec![ModelAuth::DeviceLogin, ModelAuth::Other]
    );
    let state: ModelLoginState = serde_json::from_value(json!({"state": "slow_down"})).unwrap();
    assert_eq!(state.state, LoginState::Other);
    let test: ModelTest = serde_json::from_value(json!({"result": "throttled"})).unwrap();
    assert_eq!(test.result, ModelTestResult::Other);
    let check: KeyCheck = serde_json::from_value(json!({"check": "later"})).unwrap();
    assert_eq!(check.check, KeyCheckResult::Other);
    let failure: AgentModelsFailure = serde_json::from_value(json!("teapot")).unwrap();
    assert_eq!(failure, AgentModelsFailure::Other);
}

fn provider(id: &str, auth: Vec<ModelAuth>) -> ModelProvider {
    ModelProvider {
        id: id.into(),
        name: id.to_uppercase(),
        recommended: false,
        auth,
        connected: false,
        account: None,
    }
}

fn api_key(url: Option<&str>) -> ModelAuth {
    ModelAuth::ApiKey {
        key_url: url.map(Into::into),
        key_prefix: Some("sk-".into()),
        base_url: None,
    }
}

#[test]
fn catalog_only_lets_https_key_pages_through() {
    let good = ModelCatalog {
        providers: vec![provider(
            "a",
            vec![api_key(Some("https://example.com/keys"))],
        )],
        current: None,
    };
    assert!(good.check().is_ok());
    for url in [
        "http://example.com/keys",
        "javascript:alert(1)",
        "wonton://settings",
        "https://",
        "https://example.com/a b",
    ] {
        let bad = ModelCatalog {
            providers: vec![provider("a", vec![api_key(Some(url))])],
            current: None,
        };
        assert!(bad.check().is_err(), "{url}");
    }
}

#[test]
fn catalog_rejects_duplicate_or_blank_providers() {
    let duplicate = ModelCatalog {
        providers: vec![provider("a", vec![]), provider("a", vec![])],
        current: None,
    };
    assert!(duplicate.check().is_err());
    let blank = ModelCatalog {
        providers: vec![provider(" ", vec![])],
        current: None,
    };
    assert!(blank.check().is_err());
    let blank_current = ModelCatalog {
        providers: vec![],
        current: Some(ModelChoice {
            provider: "a".into(),
            model: String::new(),
        }),
    };
    assert!(blank_current.check().is_err());
}

#[test]
fn login_url_must_be_https_and_interval_is_clamped() {
    let login = |url: &str, interval| ModelLogin {
        session: "s1".into(),
        url: url.into(),
        code: Some("ABCD-EFGH".into()),
        expires_in: 900,
        interval,
    };
    assert_eq!(
        login("https://example.com/device", 0)
            .checked()
            .unwrap()
            .interval,
        MIN_LOGIN_POLL_SECS
    );
    assert_eq!(
        login("https://example.com/device", 600)
            .checked()
            .unwrap()
            .interval,
        MAX_LOGIN_POLL_SECS
    );
    assert!(login("intent://example.com", 5).checked().is_err());
}

#[test]
fn model_list_is_bounded() {
    let many = ModelList {
        models: (0..=MAX_PROVIDER_MODELS)
            .map(|i| ModelOption {
                id: format!("m{i}"),
                name: None,
                recommended: false,
            })
            .collect(),
    };
    assert!(many.check().is_err());
}

#[test]
fn paths_encode_ids_into_single_segments() {
    assert_eq!(
        agent_models_path("com.nousresearch.hermes", "alice"),
        "/_wonton/api/bots/com.nousresearch.hermes/alice/models"
    );
    assert_eq!(
        agent_model_list_path("p", "b", "custom:local/1"),
        "/_wonton/api/bots/p/b/models/providers/custom%3Alocal%2F1/models"
    );
    assert_eq!(
        agent_model_login_finish_path("p", "b", "a/b"),
        "/_wonton/api/bots/p/b/models/logins/a%2Fb/finish"
    );
    assert_eq!(
        agent_model_current_path("p", "b"),
        "/_wonton/api/bots/p/b/models/current"
    );
    assert_eq!(
        agent_model_test_path("p", "b"),
        "/_wonton/api/bots/p/b/models/test"
    );
    assert_eq!(
        agent_model_login_start_path("p", "b", "nous"),
        "/_wonton/api/bots/p/b/models/providers/nous/login"
    );
}

#[test]
fn save_key_request_never_prints_the_key() {
    let body: SaveModelKey =
        serde_json::from_value(json!({"key": "sk-live-123", "base_url": "https://example.com/v1"}))
            .unwrap();
    assert_eq!(body.key.expose(), "sk-live-123");
    assert!(!format!("{body:?}").contains("sk-live-123"));
}
