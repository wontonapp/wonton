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
}

#[test]
fn the_ask_every_update_switch_keeps_its_wire_format() {
    let switch = AskEveryUpdate { enabled: true };
    assert_eq!(json!(switch), json!({"enabled": true}));
    assert_eq!(round_trip(&switch), switch);
}
