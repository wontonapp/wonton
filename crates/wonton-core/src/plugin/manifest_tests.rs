use serde_json::Value;

use super::*;
use crate::plugin::install_notes::MAX_INSTALL_TIMEOUT_SECS;

#[test]
fn plugin_ids_cannot_occupy_bot_web_host_labels() {
    let bot = wonton_protocol::BotRef {
        plugin_id: "com.example.agent".into(),
        bot: "alice".into(),
    };
    let label = wonton_protocol::bot_web_label(&bot);
    assert_eq!(label, "bot--alice-com-example-agent-f4debdbf");
    assert!(PluginId::parse(&label).is_err());
    assert!(PluginId::parse(&format!("{label}.app")).is_err());
    assert!(PluginId::parse(&format!("bot--.{}", &label[5..])).is_err());
    assert!(PluginId::parse("bot-.example").is_err());
    assert_eq!(
        PluginId::parse("bot.example").unwrap().slug(),
        "bot-example"
    );
}

#[test]
fn webui_accounts_is_left_out_when_not_declared() {
    let webui: WebUi = serde_json::from_value(serde_json::json!({
        "service": "web",
        "port": 8080
    }))
    .unwrap();
    assert_eq!(webui.accounts, None);
    let encoded = serde_json::to_value(&webui).unwrap();
    assert!(encoded.get("accounts").is_none());
    assert_eq!(serde_json::from_value::<WebUi>(encoded).unwrap(), webui);
}

#[test]
fn webui_accounts_round_trips_in_manifest() {
    let manifest: Manifest = serde_json::from_value(serde_json::json!({
        "spec": "0.13",
        "id": "com.example.web",
        "version": "1.0.0",
        "kind": "app",
        "title": {"en": "Example"},
        "description": {"en": "Example"},
        "permissions": [],
        "webui": {"service": "web", "port": 8080, "accounts": "per_user"}
    }))
    .unwrap();
    manifest.validate(&Languages::default()).unwrap();
    assert_eq!(
        manifest.webui.as_ref().unwrap().accounts,
        Some(Accounts::PerUser)
    );
    let encoded = serde_json::to_value(&manifest).unwrap();
    assert_eq!(encoded["webui"]["accounts"], "per_user");
    let decoded: Manifest = serde_json::from_value(encoded).unwrap();
    assert_eq!(decoded.webui, manifest.webui);
}

#[test]
fn webui_rejects_unknown_account_models_and_the_removed_friends_flag() {
    for webui in [
        serde_json::json!({"service": "web", "port": 8080, "accounts": "shared"}),
        serde_json::json!({"service": "web", "port": 8080, "accounts": true}),
        serde_json::json!({"service": "web", "port": 8080, "friends": true}),
    ] {
        assert!(serde_json::from_value::<WebUi>(webui).is_err());
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
        "devices.disk",
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

#[test]
fn the_install_deadline_is_optional_and_bounded() {
    let manifest = |seconds: Value| -> Manifest {
        let mut value = serde_json::json!({
            "spec": "0.14",
            "id": "com.example.slow",
            "version": "1.0.0",
            "kind": "app",
            "title": {"en": "Example"},
            "description": {"en": "Example"},
            "permissions": []
        });
        if !seconds.is_null() {
            value["install_timeout_secs"] = seconds;
        }
        serde_json::from_value(value).unwrap()
    };
    assert_eq!(manifest(Value::Null).install_timeout_secs, None);
    for good in [60, 900, MAX_INSTALL_TIMEOUT_SECS] {
        manifest(serde_json::json!(good))
            .validate(&Languages::default())
            .unwrap();
    }
    for bad in [59, MAX_INSTALL_TIMEOUT_SECS + 1] {
        let error = manifest(serde_json::json!(bad))
            .validate(&Languages::default())
            .unwrap_err();
        assert!(
            error.to_string().contains("install_timeout_secs"),
            "{error}"
        );
    }
}
