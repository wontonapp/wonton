//! 长按菜单项的往返。

use super::super::MessageAction;

/// 契约：编辑菜单项带着被编辑消息的原文往返，壳靠它预填输入框。
#[test]
fn edit_action_carries_the_original_text() {
    let action = MessageAction::Edit {
        text: "原来的话".into(),
    };
    let json = serde_json::json!({"type": "edit", "text": "原来的话"});
    assert_eq!(serde_json::to_value(&action).unwrap(), json);
    assert_eq!(
        serde_json::from_value::<MessageAction>(json).unwrap(),
        action
    );
}
