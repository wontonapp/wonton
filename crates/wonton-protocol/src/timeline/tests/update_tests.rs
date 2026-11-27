//! 一批时间线的往返：增量的每种变化、编辑态、加密入口和旧核心缺的字段。

use super::super::{
    BackPagination, KeyNotice, MessageContent, TimelineContent, TimelineDiff, TimelineUpdate,
    TimelineViewport,
};
use super::test_support::{item, message, round_trip, sample_items, text_with_file};
use crate::{ComposerEdit, SendState};

/// 契约：时间线增量的每种变化和每种条目都能经 JSON 往返；媒体句柄不进 Debug 输出。
#[test]
fn timeline_updates_round_trip_through_json() {
    let items = sample_items();
    let update = TimelineUpdate {
        unread_jump: None,
        diffs: vec![
            TimelineDiff::Reset {
                items: items.clone(),
            },
            TimelineDiff::Append { items },
            TimelineDiff::Set {
                index: 2,
                item: item("3", message(text_with_file(), SendState::Sent)),
            },
            TimelineDiff::Insert {
                index: 0,
                item: item("0", message(MessageContent::Redacted, SendState::Sent)),
            },
            TimelineDiff::Remove { index: 1 },
            TimelineDiff::Truncate { length: 3 },
            TimelineDiff::PopFront,
            TimelineDiff::Clear,
        ],
        back_pagination: BackPagination::ReachedStart,
        viewport: TimelineViewport::Away { new_messages: 3 },
        scroll_to_bottom: false,
        anchor: Some("5".into()),
        key_notice: Some(KeyNotice::EnterRecoveryKey),
        confirmations: Vec::new(),
        side_chat: None,
        widgets: vec![crate::RoomWidget {
            tab: crate::TabRef::Widget {
                plugin_id: "app.call".into(),
                room_id: "!room:home.example".into(),
            },
            title: "通话".into(),
            icon: None,
            entry: crate::WidgetEntry::VideoCall,
        }],
        call_active: true,
        room_bots: Some(vec!["@hermes:wonton.localhost".into()]),
        typing: vec!["Hermes".into()],
        commands: vec![crate::RoomCommand {
            bot_user_id: "@hermes:wonton.localhost".into(),
            bot_name: "Hermes".into(),
            name: "model".into(),
            description: "换模型".into(),
            hint: Some("模型名".into()),
            text: "/model".into(),
        }],
        commands_omitted: 2,
        focus: Some(crate::TimelineFocus {
            item_id: "3".into(),
            highlight: true,
        }),
        jump: crate::JumpState::Loading,
        back_to: Some("5".into()),
        editing: Some(ComposerEdit {
            item_id: "3".into(),
            event_id: "$event".into(),
            summary: "原来的话".into(),
        }),
        focus_revision: 3,
    };
    assert_eq!(round_trip(&update), update);
    let value = serde_json::to_value(&update).unwrap();
    assert_eq!(value["editing"]["item_id"], "3");
    assert_eq!(value["editing"]["summary"], "原来的话");
    assert!(
        !format!("{update:?}").contains("mxc://"),
        "Debug 不能输出媒体句柄"
    );
}

/// 契约：没在编辑时 `editing` 是 null，老核心不写这个字段时也照样解析成没在编辑。
#[test]
fn editing_is_null_until_somebody_starts_an_edit() {
    let mut update = TimelineUpdate {
        unread_jump: None,
        diffs: Vec::new(),
        back_pagination: BackPagination::ReachedStart,
        viewport: TimelineViewport::Bottom,
        scroll_to_bottom: false,
        anchor: None,
        key_notice: None,
        confirmations: Vec::new(),
        side_chat: None,
        widgets: Vec::new(),
        call_active: false,
        room_bots: None,
        typing: Vec::new(),
        commands: Vec::new(),
        commands_omitted: 0,
        focus: None,
        jump: crate::JumpState::Idle,
        back_to: None,
        editing: None,
        focus_revision: 0,
    };
    let mut value = serde_json::to_value(&update).unwrap();
    assert_eq!(value["editing"], serde_json::Value::Null);
    assert_eq!(
        serde_json::from_value::<TimelineUpdate>(value.clone()).unwrap(),
        update
    );
    value.as_object_mut().unwrap().remove("editing");
    update.editing = None;
    assert_eq!(
        serde_json::from_value::<TimelineUpdate>(value).unwrap(),
        update
    );
}

/// 契约：加密入口的每个取值都能经 JSON 往返，不认识的取值落到 `Other`，没有入口时整个字段可以缺。
#[test]
fn key_notices_round_trip_and_unknown_ones_fall_back() {
    for notice in [
        KeyNotice::EnterRecoveryKey,
        KeyNotice::VerifyWithOtherDevice,
    ] {
        assert_eq!(round_trip(&notice), notice);
    }
    assert_eq!(
        serde_json::from_value::<KeyNotice>(serde_json::json!("another")).unwrap(),
        KeyNotice::Other
    );
    let empty = || TimelineUpdate {
        unread_jump: None,
        diffs: Vec::new(),
        back_pagination: BackPagination::Idle,
        viewport: TimelineViewport::Bottom,
        scroll_to_bottom: true,
        anchor: None,
        key_notice: Some(KeyNotice::VerifyWithOtherDevice),
        confirmations: Vec::new(),
        side_chat: None,
        widgets: Vec::new(),
        call_active: false,
        room_bots: None,
        typing: Vec::new(),
        commands: Vec::new(),
        commands_omitted: 0,
        focus: None,
        jump: crate::JumpState::Idle,
        back_to: None,
        editing: None,
        focus_revision: 0,
    };
    let mut value = serde_json::to_value(empty()).unwrap();
    value.as_object_mut().unwrap().remove("key_notice");
    value.as_object_mut().unwrap().remove("room_bots");
    value.as_object_mut().unwrap().remove("jump");
    value.as_object_mut().unwrap().remove("focus_revision");
    let update: TimelineUpdate = serde_json::from_value(value).unwrap();
    assert_eq!(update.key_notice, None);
    // 没有这一字段的旧核心发来的一批照旧收下，在场的 AI 落到“不确定”。
    assert_eq!(update.room_bots, None);
    // 旧核心不带跳转序号，按 0 算。
    assert_eq!(update.focus_revision, 0);
}

/// 契约：旧核心发来的批次没有 `unread_jump` 和 `mentions_me` 时按“没有胶囊、没提到我”收下。
#[test]
fn old_updates_without_unread_fields_still_deserialize() {
    let mut value = serde_json::to_value(TimelineUpdate {
        diffs: Vec::new(),
        back_pagination: BackPagination::Idle,
        viewport: TimelineViewport::Bottom,
        scroll_to_bottom: false,
        anchor: None,
        key_notice: None,
        confirmations: Vec::new(),
        side_chat: None,
        widgets: Vec::new(),
        call_active: false,
        room_bots: None,
        typing: Vec::new(),
        commands: Vec::new(),
        commands_omitted: 0,
        focus: None,
        jump: crate::JumpState::Idle,
        back_to: None,
        editing: None,
        focus_revision: 0,
        unread_jump: None,
    })
    .unwrap();
    value.as_object_mut().unwrap().remove("unread_jump");
    let update: TimelineUpdate = serde_json::from_value(value).unwrap();
    assert_eq!(update.unread_jump, None);

    let mut content =
        serde_json::to_value(message(MessageContent::Redacted, SendState::Sent)).unwrap();
    let removed = content["message"]
        .as_object_mut()
        .unwrap()
        .remove("mentions_me");
    assert_eq!(removed, Some(serde_json::Value::Bool(false)));
    let parsed: TimelineContent = serde_json::from_value(content).unwrap();
    let TimelineContent::Message { message } = parsed else {
        panic!("不是消息");
    };
    assert!(!message.mentions_me);
}
