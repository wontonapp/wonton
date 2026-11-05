use super::*;

#[test]
fn agent_login_defaults_to_empty_and_requires_egress() {
    let base = r#"{
        "spec": "0.6",
        "id": "com.example.agent",
        "version": "1.0.0",
        "kind": "agent",
        "title": {"zh": "代理", "en": "Agent"},
        "description": {"zh": "描述", "en": "Description"},
        "agent": {"protocol": "acp"},
        "permissions": []
    }"#;
    let manifest: Manifest = serde_json::from_str(base).unwrap();
    assert!(manifest.agent.unwrap().login.is_empty());

    let login = base.replace(
        "\"agent\": {\"protocol\": \"acp\"}",
        "\"agent\": {\"protocol\": \"acp\", \"login\": [{\"id\": \"chatgpt\", \"title\": {\"zh\": \"账号\", \"en\": \"Account\"}}]}",
    );
    let error = serde_json::from_str::<Manifest>(&login)
        .unwrap()
        .validate()
        .unwrap_err();
    assert!(error.to_string().contains("network.egress"), "{error}");
}

#[test]
fn agent_login_ids_are_valid_and_unique() {
    let base = r#"{
        "spec": "0.6",
        "id": "com.example.agent",
        "version": "1.0.0",
        "kind": "agent",
        "title": {"zh": "代理", "en": "Agent"},
        "description": {"zh": "描述", "en": "Description"},
        "agent": {"protocol": "acp", "login": LOGIN},
        "permissions": ["network.egress"]
    }"#;
    for login in [
        r#"[{"id":"bad.id","title":{"zh":"账号","en":"Account"}}]"#,
        r#"[{"id":"chatgpt","title":{"zh":"账号","en":"Account"}},{"id":"chatgpt","title":{"zh":"另一个","en":"Another"}}]"#,
    ] {
        let error = serde_json::from_str::<Manifest>(&base.replace("LOGIN", login))
            .unwrap()
            .validate()
            .unwrap_err();
        assert!(error.to_string().contains("agent.login"), "{error}");
    }
    let valid = base.replace(
        "LOGIN",
        r#"[{"id":"chatgpt","title":{"zh":"账号","en":"Account"}}]"#,
    );
    assert!(
        serde_json::from_str::<Manifest>(&valid)
            .unwrap()
            .validate()
            .is_ok()
    );
}
