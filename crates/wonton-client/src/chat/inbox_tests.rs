//! 收件箱的合成：核心的列表加确认卡片列表，壳拿到的铃铛计数和条目。

use wonton_protocol::{
    ConfirmationCard, ConfirmationState, ConfirmationView, ConfirmationsViewModel, InboxCounts,
    InboxList, InboxListItem, InboxRecipient, InboxSender, InboxSenderKind, InboxStatus, InboxType,
};

use super::compose;

fn confirm_item(card: &str, status: InboxStatus) -> InboxListItem {
    InboxListItem {
        id: format!("c-{card}"),
        kind: InboxType::Confirm,
        status,
        sender: InboxSender {
            kind: InboxSenderKind::Agent,
            id: "app.hermes".into(),
            display_name: "Hermes".into(),
            avatar: None,
        },
        recipient: InboxRecipient::Person {
            username: "owner".into(),
        },
        title: "装 Immich？".into(),
        body: String::new(),
        level: None,
        created_at_ms: 1,
        expires_at_ms: Some(600_001),
        snooze_until_ms: None,
        needs_answer: status == InboxStatus::Pending,
        unread: true,
        read_by: Vec::new(),
        first_question: None,
        question_count: 0,
        action_count: 0,
        progress: None,
    }
}

fn card(id: &str, state: ConfirmationState) -> ConfirmationView {
    let card: ConfirmationCard = serde_json::from_value(serde_json::json!({
        "id": id,
        "state": serde_json::to_value(&state).unwrap(),
        "title": "装 Immich？",
        "lines": [],
        "choices": [{"answer": "approve", "label": "允许"}, {"answer": "reject", "label": "拒绝"}],
        "expires_at_ms": 600_001u64,
    }))
    .unwrap();
    ConfirmationView {
        card,
        sending: None,
        sending_choice_id: None,
        problem: None,
        window: None,
    }
}

fn list(items: Vec<InboxListItem>, needing: u32, revision: u64) -> InboxList {
    InboxList {
        items,
        counts: InboxCounts {
            needing,
            unread_notices: 0,
        },
        revision,
    }
}

fn loaded(
    pending: Vec<ConfirmationView>,
    finished: Vec<ConfirmationView>,
) -> ConfirmationsViewModel {
    ConfirmationsViewModel {
        pending,
        finished,
        ..Default::default()
    }
}

#[test]
fn nothing_to_answer_means_no_count_and_no_entries() {
    let view = compose(
        Some(&list(Vec::new(), 0, 3)),
        &loaded(Vec::new(), Vec::new()),
        None,
    );
    assert_eq!(view.list.counts.needing, 0);
    assert!(view.entries.is_empty());
    assert!(!view.loading);
    assert!(view.problem.is_none());
}

#[test]
fn a_pending_confirmation_is_an_entry_that_can_be_answered() {
    let view = compose(
        Some(&list(vec![confirm_item("k1", InboxStatus::Pending)], 1, 4)),
        &loaded(vec![card("k1", ConfirmationState::Pending)], Vec::new()),
        None,
    );
    assert_eq!(view.list.counts.needing, 1);
    assert_eq!(view.entries.len(), 1);
    let entry = &view.entries[0];
    assert_eq!(entry.card.as_ref().unwrap().card.id, "k1");
    assert_eq!(entry.card.as_ref().unwrap().card.choices.len(), 2);
}

#[test]
fn an_answered_confirmation_no_longer_counts() {
    let view = compose(
        Some(&list(vec![confirm_item("k1", InboxStatus::Handled)], 0, 5)),
        &loaded(Vec::new(), vec![card("k1", ConfirmationState::Rejected)]),
        None,
    );
    assert_eq!(view.list.counts.needing, 0);
    assert_eq!(view.entries[0].item.status, InboxStatus::Handled);
    assert_eq!(
        view.entries[0].card.as_ref().unwrap().card.state,
        ConfirmationState::Rejected
    );
}

#[test]
fn before_the_first_list_the_count_is_unknown_not_zero() {
    let view = compose(None, &loaded(Vec::new(), Vec::new()), None);
    assert!(view.loading, "还没取到过就不能当成“没有要回答的”");
}

#[test]
fn a_card_missing_from_the_card_list_leaves_only_the_summary() {
    let view = compose(
        Some(&list(vec![confirm_item("k2", InboxStatus::Pending)], 1, 6)),
        &loaded(Vec::new(), Vec::new()),
        None,
    );
    assert!(view.entries[0].card.is_none());
    assert_eq!(view.list.counts.needing, 1);
}
