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

#[test]
fn plugin_id_must_be_reverse_dns() {
    assert_eq!(
        PluginId::parse("app.immich.photos").unwrap().slug(),
        "app-immich-photos"
    );
    for bad in [
        "immich",
        "App.immich",
        "app..x",
        "app.-x",
        "app.x-",
        "app_x.y",
    ] {
        assert!(PluginId::parse(bad).is_err(), "{bad} 应被拒绝");
    }
    let long = format!("app.{}", "a".repeat(60));
    assert!(PluginId::parse(&long).is_err());
}

#[test]
fn permissions_round_trip_and_unknown_is_rejected() {
    for text in [
        "storage.share.photos:read",
        "storage.home:write",
        "models",
        "matrix.bot",
        "notify",
        "network.egress",
        "network.lan",
        "network.host",
        "cap.NET_ADMIN",
        "host.pid",
        "host.ipc",
        "security.opt",
        "privileged",
        "devices.gpu",
        "devices./dev/net/tun",
        "docker.socket",
    ] {
        let permission: Permission = text.parse().unwrap();
        assert_eq!(permission.to_string(), text);
    }
    for bad in [
        "storage.share.photos",
        "storage.share.Photos:read",
        "root",
        "devices.usb",
    ] {
        assert!(bad.parse::<Permission>().is_err(), "{bad} 应被拒绝");
    }
    assert!(Permission::DockerSocket.is_high());
    assert!(!Permission::Models.is_high());
}

#[test]
fn semver_is_checked() {
    for good in ["0.1.0", "2.0.0", "1.2.3-beta.1", "1.2.3+build.5"] {
        assert!(is_semver(good), "{good}");
    }
    for bad in ["1.2", "v1.2.3", "01.2.3", "1.2.x", "1.2.3-é"] {
        assert!(!is_semver(bad), "{bad}");
    }
}
