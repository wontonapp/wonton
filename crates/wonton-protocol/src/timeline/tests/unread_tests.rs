//! 未读胶囊的往返和不认识的失败原因。

use super::super::{UnreadJump, UnreadJumpState, UnreadJumpUnavailable};
use super::test_support::round_trip;

/// 契约：胶囊的状态能经 JSON 往返；空锚点不写字段，缺字段按空；不认识的失败原因落 `Other`。
#[test]
fn unread_jump_round_trips_and_unknown_reasons_fall_back() {
    let jump = UnreadJump {
        anchor_item_id: None,
        count: 26,
        mentions: 2,
        first_mention_item_id: Some("$m".into()),
        state: UnreadJumpState::Loading,
    };
    assert_eq!(round_trip(&jump), jump);
    let value = serde_json::to_value(&jump).unwrap();
    assert!(value.get("anchor_item_id").is_none());

    let missing: UnreadJump = serde_json::from_value(serde_json::json!({
        "count": 1,
        "mentions": 0,
        "state": {"type": "unavailable", "reason": "offline"},
    }))
    .unwrap();
    assert_eq!(missing.anchor_item_id, None);
    assert_eq!(
        missing.state,
        UnreadJumpState::Unavailable {
            reason: UnreadJumpUnavailable::Offline
        }
    );

    let unknown: UnreadJumpState = serde_json::from_value(serde_json::json!({
        "type": "unavailable",
        "reason": "another",
    }))
    .unwrap();
    assert_eq!(
        unknown,
        UnreadJumpState::Unavailable {
            reason: UnreadJumpUnavailable::Other
        }
    );
}
