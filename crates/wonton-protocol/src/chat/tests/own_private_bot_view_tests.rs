use serde_json::json;

use crate::{OwnBotFailure, OwnBotProblem, OwnPrivateBotView, PrivateBotRoomProblem};

#[test]
fn unknown_private_bot_view_facts_remain_uncertain() {
    let future: OwnPrivateBotView = serde_json::from_value(json!({
        "state": "future_state", "new_field": "ignored"
    }))
    .unwrap();
    assert_eq!(future, OwnPrivateBotView::Other);

    let problem: OwnBotProblem = serde_json::from_value(json!({
        "category": "future_failure", "kind": "future_kind",
        "status": 503, "detail": "diagnostic"
    }))
    .unwrap();
    assert_eq!(problem.category, OwnBotFailure::Other);
    assert_eq!(problem.kind.as_deref(), Some("future_kind"));
    assert_eq!(problem.status, Some(503));

    let ready: OwnPrivateBotView = serde_json::from_value(json!({
        "state": "ready", "bot": {"plugin_id": "app.bot", "bot": "mia"},
        "user_id": "@mia-bot:test", "home_room": null, "room_problem": "future_problem"
    }))
    .unwrap();
    assert!(matches!(
        ready,
        OwnPrivateBotView::Ready {
            home_room: None,
            room_problem: Some(PrivateBotRoomProblem::Other),
            ..
        }
    ));
}
