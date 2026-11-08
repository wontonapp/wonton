use super::*;

#[test]
fn webui_friends_defaults_to_false() {
    let webui: WebUi = serde_json::from_value(serde_json::json!({
        "service": "web",
        "port": 8080
    }))
    .unwrap();
    assert!(!webui.friends);
    let encoded = serde_json::to_value(&webui).unwrap();
    assert_eq!(encoded["friends"], false);
    assert_eq!(serde_json::from_value::<WebUi>(encoded).unwrap(), webui);
}

#[test]
fn webui_friends_round_trips_in_manifest() {
    for friends in [false, true] {
        let manifest: Manifest = serde_json::from_value(serde_json::json!({
            "spec": "0.9",
            "id": "com.example.web",
            "version": "1.0.0",
            "kind": "app",
            "title": {"en": "Example"},
            "description": {"en": "Example"},
            "permissions": [],
            "webui": {"service": "web", "port": 8080, "friends": friends}
        }))
        .unwrap();
        manifest.validate(&Languages::default()).unwrap();
        assert_eq!(manifest.webui.as_ref().unwrap().friends, friends);
        let encoded = serde_json::to_value(&manifest).unwrap();
        assert_eq!(encoded["webui"]["friends"], friends);
        let decoded: Manifest = serde_json::from_value(encoded.clone()).unwrap();
        assert_eq!(decoded.webui, manifest.webui);
        assert_eq!(serde_json::to_value(decoded).unwrap(), encoded);
    }
}

#[test]
fn webui_friends_rejects_non_boolean_values() {
    for invalid in [
        serde_json::json!(null),
        serde_json::json!("true"),
        serde_json::json!("false"),
        serde_json::json!(0),
        serde_json::json!(1),
        serde_json::json!([]),
        serde_json::json!({}),
    ] {
        assert!(
            serde_json::from_value::<WebUi>(serde_json::json!({
                "service": "web", "port": 8080, "friends": invalid
            }))
            .is_err()
        );
    }
}

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
fn bot_chat_layout_defaults_to_bubbles_and_accepts_only_the_two_forms() {
    let base = serde_json::json!({
        "spec": "0.6", "id": "com.example.bot", "version": "1.0.0", "kind": "agent",
        "title": {"zh": "代理", "en": "Agent"},
        "description": {"zh": "描述", "en": "Description"},
        "permissions": ["matrix.bot"],
        "agent": {"protocol": "matrix"},
        "bot": {}
    });
    let manifest: Manifest = serde_json::from_value(base.clone()).unwrap();
    manifest.validate(&Languages::default()).unwrap();
    assert_eq!(manifest.bot.clone().unwrap().chat_layout, ChatForm::Bubbles);
    for (form, expected) in [
        ("bubbles", ChatForm::Bubbles),
        ("document", ChatForm::Document),
    ] {
        let mut value = base.clone();
        value["bot"] = serde_json::json!({"chat_layout": form});
        let manifest: Manifest = serde_json::from_value(value).unwrap();
        manifest.validate(&Languages::default()).unwrap();
        assert_eq!(manifest.bot.unwrap().chat_layout, expected);
    }
    // 比这个核心新的取值解析成 `Other`，装上以后不会按默认的 `Bubbles` 静默处理，校验时就报。
    let mut invalid: Manifest = serde_json::from_value(base).unwrap();
    invalid.bot.as_mut().unwrap().chat_layout = ChatForm::Other;
    let error = invalid.validate(&Languages::default()).unwrap_err();
    assert!(error.to_string().contains("bot.chat_layout"), "{error}");
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
        "devices.serial",
        "devices.usb",
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
        "devices.",
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
