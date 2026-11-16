//! 设置进出既有接口的契约：安装、升级预览与请求，确认卡片与回答，插件详情。

use super::*;
use crate::{Answer, ConfirmationAnswer, ConfirmationCard, ConfirmationState};

fn field(name: &str, value: Option<SettingValue>) -> PluginSettingField {
    PluginSettingField {
        name: name.into(),
        title: "预算".into(),
        description: None,
        required: true,
        kind: PluginSettingKind::Text {
            min_length: None,
            max_length: None,
            pattern: None,
        },
        value,
        default: None,
        services: vec!["server".into()],
        problem: None,
    }
}

fn form() -> SettingsForm {
    SettingsForm {
        fields: vec![
            field(
                "budget",
                Some(SettingValue::Text {
                    value: "household".into(),
                }),
            ),
            field("months", None),
        ],
    }
}

#[test]
fn install_previews_and_requests_carry_settings() {
    let mut preview = preview();
    preview.settings = Some(form());
    assert_eq!(round_trip(&preview).settings, preview.settings);

    let mut request: InstallRequest = serde_json::from_value(json!({
        "source": "github:example/wonton-tools@v1.0.0",
        "digest": "sha256:abc",
        "grant": []
    }))
    .unwrap();
    assert!(request.settings.is_empty());
    request.settings.insert(
        "budget".into(),
        SettingValue::Text {
            value: "personal".into(),
        },
    );
    assert_eq!(round_trip(&request).settings, request.settings);

    let upgrade = UpgradeRequest {
        devices: Vec::new(),
        source: None,
        digest: "sha256:abc".into(),
        grant: Vec::new(),
        enabled_permissions: Vec::new(),
        secrets: Default::default(),
        settings: request.settings.clone(),
        lan_webui: false,
    };
    assert_eq!(round_trip(&upgrade).settings, upgrade.settings);
}

#[test]
fn confirmation_cards_and_answers_carry_settings() {
    let mut card = ConfirmationCard {
        id: "c-1".into(),
        state: ConfirmationState::Pending,
        title: "Hermes 想安装 Actual".into(),
        lines: Vec::new(),
        choices: Vec::new(),
        expires_at_ms: 1_793_000_000_000,
        window: None,
        plugin_id: Some("app.actual".into()),
        secret_inputs: Vec::new(),
        settings: Some(form()),
        permissions: Vec::new(),
        devices: Vec::new(),
        requester: None,
        origin: None,
        audience: crate::CardAudience::Keeper,
        answered_by: None,
        category: None,
    };
    let back: ConfirmationCard = round_trip(&card);
    assert_eq!(back.settings, card.settings);

    card.permissions = vec![crate::PermissionLine {
        permission: "devices.usb".into(),
        text: String::new(),
        risk: Some(crate::PermissionRisk::VeryHigh),
        notice: None,
        supported: true,
        added: false,
        optional: false,
        enabled: false,
        off_loss: None,
        opened: None,
    }];
    card.devices = vec![crate::PluginDevice {
        id: "disk:/dev/disk/by-id/test".into(),
        name: "disk".into(),
        target: "/dev/sda".into(),
        service: "smart".into(),
        optional: true,
        kind: crate::PluginDeviceKind::Disk,
        system: true,
        chosen: true,
    }];
    let wire = serde_json::to_value(&card).unwrap();
    assert_eq!(wire["permissions"][0]["permission"], "devices.usb");
    assert_eq!(wire["permissions"][0]["risk"], "very_high");
    assert_eq!(wire["devices"][0]["kind"], "disk");
    assert_eq!(wire["devices"][0]["system"], true);
    assert_eq!(wire["devices"][0]["chosen"], true);
    assert_eq!(round_trip(&card), card);
    let answer = ConfirmationAnswer {
        answer: Answer::Approve,
        choice_id: None,
        secrets: Default::default(),
        settings: [("months".to_owned(), SettingValue::Integer { value: 6 })].into(),
        devices: Vec::new(),
    };
    assert_eq!(round_trip(&answer).settings, answer.settings);

    // 没有设置的卡片和旧 App 的回答照常解析。
    let mut old = card;
    old.settings = None;
    let json = json!(old);
    assert_eq!(json.get("settings"), None);
    let old_answer: ConfirmationAnswer = serde_json::from_value(json!({
        "answer": "approve",
    }))
    .unwrap();
    assert!(old_answer.settings.is_empty());
}

#[test]
fn detail_tells_whether_there_are_settings_and_parses_without_the_flag() {
    let plain = detail();
    assert!(!round_trip(&plain).has_settings);

    let with = PluginDetail {
        has_settings: true,
        ..plain
    };
    assert!(round_trip(&with).has_settings);

    // 旧核心的详情没有这个字段：按没有设置算。
    let mut old = serde_json::to_value(detail()).unwrap();
    old.as_object_mut().unwrap().remove("has_settings");
    let parsed: PluginDetail = serde_json::from_value(old).unwrap();
    assert!(!parsed.has_settings);
}

#[test]
fn settings_view_and_update_round_trip() {
    let view = PluginSettingsView {
        schema_digest: "sha256:ab".into(),
        form: form(),
        changed_at_ms: None,
    };
    assert_eq!(round_trip(&view), view);

    let update = PluginSettingsUpdate {
        schema_digest: "sha256:ab".into(),
        values: [("months".to_owned(), None)].into(),
    };
    assert_eq!(round_trip(&update).values.len(), 1);
    assert_eq!(round_trip(&update).values["months"], None);
}
