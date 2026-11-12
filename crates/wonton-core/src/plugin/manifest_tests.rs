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
        .validate(&Languages::default())
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
            .validate(&Languages::default())
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
            .validate(&Languages::default())
            .is_ok()
    );
}

#[test]
fn bot_avatars_accept_exactly_the_four_activity_keys() {
    let base = serde_json::json!({
        "spec": "0.6", "id": "com.example.bot", "version": "1.0.0", "kind": "agent",
        "title": {"zh": "代理", "en": "Agent"},
        "description": {"zh": "描述", "en": "Description"},
        "permissions": ["matrix.bot"],
        "agent": {"protocol": "matrix"},
        "bot": {"avatars": {"idle": "idle.mp4", "working": "working.webp",
            "waiting": "waiting.gif", "offline": "offline.png"}}
    });
    let manifest: Manifest = serde_json::from_value(base.clone()).unwrap();
    manifest.validate(&Languages::default()).unwrap();
    assert_eq!(manifest.bot.unwrap().avatars.unwrap().len(), 4);
    for key in ["busy", "Idle", "", "other"] {
        let mut invalid = base.clone();
        invalid["bot"]["avatars"][key] = "extra.mp4".into();
        assert!(
            serde_json::from_value::<Manifest>(invalid)
                .unwrap()
                .validate(&Languages::default())
                .is_err()
        );
    }
    let mut absent = base.clone();
    absent["bot"] = serde_json::json!({});
    assert!(
        serde_json::from_value::<Manifest>(absent)
            .unwrap()
            .bot
            .unwrap()
            .avatars
            .is_none()
    );
    let mut empty = base;
    empty["bot"]["avatars"] = serde_json::json!({});
    let manifest: Manifest = serde_json::from_value(empty).unwrap();
    manifest.validate(&Languages::default()).unwrap();
    assert_eq!(manifest.bot.unwrap().avatars, Some(BTreeMap::new()));
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
