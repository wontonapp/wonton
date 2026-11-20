//! 跳转状态机（ADR-0109 §4）。

use wonton_matrix::{
    EntryKind, Message, MessageBody, OwnedEventId, OwnedUserId, SendStatus, TimelineEntry, Vector,
};
use wonton_protocol::{JumpState, TimelineScroll};

use super::*;

fn message(event_id: &str) -> Message {
    Message {
        event_id: Some(OwnedEventId::try_from(event_id).unwrap()),
        sender: OwnedUserId::try_from("@a:home.example").unwrap(),
        sender_name: "A".into(),
        is_own: false,
        timestamp_ms: 0,
        status: SendStatus::Sent,
        body: MessageBody::Text("hi".into()),
        custom: Default::default(),
        mentions: Vec::new(),
        thread_root: None,
        thread_replies: None,
        edited: false,
        read_by: Vec::new(),
        reply: None,
        agent_run: None,
        group: None,
        can_redact: false,
        reactions: Vec::new(),
        is_direct: false,
        mentions_me: false,
        sender_avatar: None,
    }
}

fn entry(item_id: &str, event_id: Option<&str>) -> TimelineEntry {
    TimelineEntry {
        id: item_id.into(),
        kind: match event_id {
            Some(event_id) => EntryKind::Message(Box::new(message(event_id))),
            None => EntryKind::Notice(String::new()),
        },
    }
}

fn entries(items: &[(&str, Option<&str>)]) -> Vector<TimelineEntry> {
    items
        .iter()
        .map(|(item_id, event_id)| entry(item_id, *event_id))
        .collect::<Vec<_>>()
        .into()
}

fn focus(jumps: &Jumps) -> Option<(String, bool)> {
    jumps
        .take_focus()
        .map(|focus| (focus.item_id, focus.highlight))
}

/// 行为：跳到的原消息在已加载的条目里，就把跳之前那条压栈、给带高亮的焦点。
#[test]
fn a_loaded_message_is_focused_and_the_origin_is_kept() {
    let jumps = Jumps::new();
    jumps.request("$1".into(), "a".into());
    jumps.begin(&entries(&[("a", Some("$0")), ("b", Some("$1"))]));
    assert_eq!(jumps.state(), JumpState::Idle);
    assert_eq!(focus(&jumps), Some(("b".into(), true)));
    assert_eq!(jumps.back().as_deref(), Some("a"));
    // 焦点只在这一批。
    assert_eq!(focus(&jumps), None);
}

/// 行为：连跳两次，回来时按后进先出，一条一条弹，高亮都是关。
#[test]
fn two_jumps_come_back_in_reverse_order() {
    let jumps = Jumps::new();
    let all = entries(&[("a", Some("$0")), ("b", Some("$1")), ("c", Some("$2"))]);
    jumps.request("$1".into(), "a".into());
    jumps.begin(&all);
    jumps.request("$2".into(), "b".into());
    jumps.begin(&all);
    assert_eq!(jumps.back().as_deref(), Some("b"));
    jumps.request_back();
    jumps.begin(&all);
    assert_eq!(focus(&jumps), Some(("b".into(), false)));
    assert_eq!(jumps.back().as_deref(), Some("a"));
    assert!(!jumps.take_bottom());
    jumps.request_back();
    jumps.begin(&all);
    assert_eq!(focus(&jumps), Some(("a".into(), false)));
    assert_eq!(jumps.back(), None);
    assert!(jumps.take_bottom());
}

/// 行为：栈已经空了再回来，就是滚到底。
#[test]
fn coming_back_with_an_empty_stack_scrolls_to_bottom() {
    let jumps = Jumps::new();
    jumps.request_back();
    jumps.begin(&entries(&[("a", Some("$0"))]));
    assert_eq!(focus(&jumps), None);
    assert!(jumps.take_bottom());
    assert!(!jumps.take_bottom());
}

/// 行为：滚到底清栈，`back_to` 随之消失。
#[test]
fn scrolling_to_bottom_clears_the_stack() {
    let jumps = Jumps::new();
    jumps.request("$0".into(), "a".into());
    jumps.begin(&entries(&[("a", Some("$0"))]));
    assert_eq!(jumps.back().as_deref(), Some("a"));
    jumps.scrolled(TimelineScroll::Bottom);
    assert_eq!(jumps.back(), None);
    assert_eq!(jumps.state(), JumpState::Idle);
}

/// 行为：栈里的条目被删了（撤回等）就从栈里去掉；都不在了，再回来就滚到底。
#[test]
fn deleted_items_leave_the_stack() {
    let jumps = Jumps::new();
    jumps.request("$1".into(), "a".into());
    jumps.begin(&entries(&[("a", Some("$0")), ("b", Some("$1"))]));
    jumps.progress(&entries(&[("b", Some("$1"))]));
    assert_eq!(jumps.back(), None);
    jumps.request_back();
    jumps.begin(&entries(&[("b", Some("$1"))]));
    assert!(jumps.take_bottom());
}

/// 行为：要跳的原消息还没加载就先 Loading，之后随某一批进来再定下焦点。
#[test]
fn an_unloaded_message_waits_and_focuses_when_it_arrives() {
    let jumps = Jumps::new();
    jumps.request("$9".into(), "a".into());
    jumps.begin(&entries(&[("a", Some("$0"))]));
    assert_eq!(jumps.state(), JumpState::Loading);
    assert_eq!(focus(&jumps), None);
    jumps.progress(&entries(&[("a", Some("$0")), ("z", Some("$9"))]));
    assert_eq!(jumps.state(), JumpState::Idle);
    assert_eq!(focus(&jumps), Some(("z".into(), true)));
    assert_eq!(jumps.back().as_deref(), Some("a"));
}

/// 行为：翻到房间开头还没有就是 Unavailable，下一次跳转回到 Loading。
#[test]
fn giving_up_marks_the_message_unavailable() {
    let jumps = Jumps::new();
    jumps.request("$9".into(), "a".into());
    jumps.begin(&entries(&[("a", Some("$0"))]));
    jumps.give_up();
    assert_eq!(jumps.state(), JumpState::Unavailable);
    jumps.request("$8".into(), "a".into());
    jumps.begin(&entries(&[("a", Some("$0"))]));
    assert_eq!(jumps.state(), JumpState::Loading);
}

/// 行为：滚一下就把 Unavailable 收起来。
#[test]
fn scrolling_back_to_a_place_clears_the_marker() {
    let jumps = Jumps::new();
    jumps.request("$9".into(), "a".into());
    jumps.begin(&entries(&[]));
    jumps.give_up();
    jumps.scrolled(TimelineScroll::Away);
    assert_eq!(jumps.state(), JumpState::Idle);
}

/// 行为：找的途中壳报位置（布局落了、翻页带动位置），不把找了一半的叫停。
#[test]
fn scrolling_during_a_search_keeps_looking() {
    let jumps = Jumps::new();
    jumps.request("$9".into(), "a".into());
    jumps.begin(&entries(&[("a", Some("$0"))]));
    jumps.scrolled(TimelineScroll::Away);
    assert_eq!(jumps.state(), JumpState::Loading);
    jumps.progress(&entries(&[("a", Some("$0")), ("z", Some("$9"))]));
    assert_eq!(jumps.state(), JumpState::Idle);
    assert_eq!(focus(&jumps), Some(("z".into(), true)));
}
