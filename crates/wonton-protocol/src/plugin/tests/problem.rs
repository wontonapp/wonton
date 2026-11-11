//! 「服务器太旧」的失败类别和“每次都问”开关的线格式。

use serde_json::json;

use super::*;

/// 契约：`min_version` 原样往返；旧的核心不写这个字段也读得懂。
#[test]
fn a_too_old_problem_keeps_the_minimum_version() {
    let problem = PluginProblem {
        reason: PluginFailure::ServerTooOld,
        message: "太旧".into(),
        min_version: Some("0.1.0-beta.5".into()),
        settings: None,
    };
    let json = serde_json::to_value(&problem).unwrap();
    assert_eq!(json["min_version"], "0.1.0-beta.5");
    assert_eq!(
        serde_json::from_value::<PluginProblem>(json).unwrap(),
        problem
    );
    let old: PluginProblem =
        serde_json::from_value(json!({"reason": "upstream", "message": "x"})).unwrap();
    assert_eq!(old.min_version, None);
    assert_eq!(old.settings, None);
}

/// 契约：设置校验失败带字段明细，壳按类别和字段标红；明细只在这两个类别上出现。
#[test]
fn settings_problems_carry_their_details() {
    let problem = PluginProblem {
        reason: PluginFailure::SettingsInvalid,
        message: "设置不合法".into(),
        min_version: None,
        settings: Some(SettingsProblem {
            fields: vec![SettingsFieldError {
                name: "months".into(),
                kind: SettingsFieldProblemKind::AboveMaximum,
                message: "最多 12".into(),
            }],
            problems: Vec::new(),
        }),
    };
    let json = serde_json::to_value(&problem).unwrap();
    assert_eq!(json["reason"], "settings_invalid");
    assert_eq!(json["settings"]["fields"][0]["name"], "months");
    assert_eq!(round_trip(&problem), problem);

    let schema = json!({
        "reason": "settings_schema_invalid",
        "message": "Schema 不合法",
        "settings": {"problems": [{"kind": "sensitive", "message": "要移到密钥"}]},
    });
    let parsed: PluginProblem = serde_json::from_value(schema).unwrap();
    assert_eq!(parsed.reason, PluginFailure::SettingsSchemaInvalid);
    assert_eq!(
        parsed.settings.unwrap().problems[0].kind,
        SettingsSchemaProblemKind::Sensitive
    );
}

#[test]
fn the_ask_every_update_switch_keeps_its_wire_format() {
    let switch = AskEveryUpdate { enabled: true };
    assert_eq!(json!(switch), json!({"enabled": true}));
    assert_eq!(round_trip(&switch), switch);
}
