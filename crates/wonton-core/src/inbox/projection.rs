//! 现有确认卡片（ADR-0016/0024）投影成收件箱里的 `confirm` 条目。
//!
//! 确认表还是权威：投影只读，状态跟着卡片变；回答问题仍走 `POST /_wonton/confirmations/{id}`。
//! 条目 id 是 `c-<卡片 id>`，核心从里面认出确认表的那一张。

use wonton_protocol::{
    Answer, CardAudience, ConfirmAction, ConfirmChoice, ConfirmPayload, ConfirmationCard,
    ConfirmationRequester, ConfirmationState, InboxAnswer, InboxAnswerAction, InboxItem,
    InboxOutcome, InboxPayload, InboxRecipient, InboxRisk, InboxSender, InboxSenderKind,
    InboxSource, InboxStatus, InboxType,
};

/// 一张确认卡片在收件箱里的样子。`keeper` 是卡片收件人的用户名（`Admins` 时忽略）。
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn project_confirmation(card: &ConfirmationCard, keeper: Option<&str>, now: u64) -> InboxItem {
    let (status, outcome, result) = match &card.state {
        ConfirmationState::Pending | ConfirmationState::Running => {
            (InboxStatus::Pending, None, None)
        }
        ConfirmationState::Succeeded { message } => (
            InboxStatus::Handled,
            Some(InboxOutcome::Answered),
            Some(message.clone()),
        ),
        ConfirmationState::Failed { message } | ConfirmationState::Uncertain { message } => (
            InboxStatus::Handled,
            Some(InboxOutcome::Answered),
            Some(message.clone()),
        ),
        ConfirmationState::Rejected => (InboxStatus::Handled, Some(InboxOutcome::Declined), None),
        ConfirmationState::Expired => (InboxStatus::Expired, None, None),
    };
    let answered = matches!(
        &card.state,
        ConfirmationState::Succeeded { .. }
            | ConfirmationState::Failed { .. }
            | ConfirmationState::Uncertain { .. }
            | ConfirmationState::Rejected
    );
    let action = if matches!(&card.state, ConfirmationState::Rejected) {
        Some(InboxAnswerAction::Deny)
    } else {
        answered.then_some(InboxAnswerAction::Approve)
    };
    InboxItem {
        id: format!("c-{}", card.id),
        kind: InboxType::Confirm,
        status,
        created_at_ms: card.expires_at_ms.saturating_sub(600_000),
        expires_at_ms: Some(card.expires_at_ms),
        sender: sender_of(card),
        recipient: match card.audience {
            CardAudience::Admins => InboxRecipient::Admins,
            _ => InboxRecipient::Person {
                username: keeper.unwrap_or_default().to_owned(),
            },
        },
        title: card.title.clone(),
        body: card
            .lines
            .iter()
            .map(|line| line.text.clone())
            .collect::<Vec<_>>()
            .join("\n"),
        source: card.origin.as_ref().map(|origin| InboxSource {
            room_id: Some(origin.room_id.clone()),
            thread_id: origin.thread_id.clone(),
            worker_id: None,
            task: None,
        }),
        level: None,
        blocking: matches!(card.state, ConfirmationState::Pending),
        dedupe_key: None,
        progress: None,
        snooze_until_ms: None,
        read_by: Vec::new(),
        answered_by: card.answered_by.clone(),
        answered_at_ms: None,
        outcome,
        result,
        raw_payload: None,
        payload: InboxPayload::Confirm(ConfirmPayload {
            title: card.title.clone(),
            body: card
                .lines
                .iter()
                .map(|line| line.text.clone())
                .collect::<Vec<_>>()
                .join("\n"),
            detail: None,
            risk: InboxRisk::High,
            choices: card
                .choices
                .iter()
                .filter_map(|choice| {
                    let action = match choice.answer {
                        Answer::Approve | Answer::ApproveWider => ConfirmAction::Approve,
                        Answer::Reject => ConfirmAction::Deny,
                        Answer::AllowWindow => ConfirmAction::Always,
                        Answer::Unknown => return None,
                    };
                    Some(ConfirmChoice {
                        action,
                        label: choice.label.clone(),
                    })
                })
                .collect(),
        }),
        answer: action.map(|action| InboxAnswer {
            answer_id: format!("confirmation-{}", card.id),
            action,
            answers: Vec::new(),
            action_id: None,
            comment: None,
            answered_at_ms: now,
            by: card.answered_by.clone(),
        }),
        superseded_answers: Vec::new(),
    }
}

fn sender_of(card: &ConfirmationCard) -> InboxSender {
    match &card.requester {
        Some(ConfirmationRequester::Bot { bot, .. }) => InboxSender {
            kind: InboxSenderKind::Agent,
            id: bot.plugin_id.clone(),
            display_name: bot.plugin_id.clone(),
            avatar: None,
        },
        Some(ConfirmationRequester::Plugin { plugin_id }) => InboxSender {
            kind: InboxSenderKind::Plugin,
            id: plugin_id.clone(),
            display_name: plugin_id.clone(),
            avatar: None,
        },
        Some(ConfirmationRequester::Other) | None => InboxSender {
            kind: InboxSenderKind::Core,
            id: "core".to_owned(),
            display_name: "云吞".to_owned(),
            avatar: None,
        },
    }
}

/// `c-<确认 id>` 里取出确认 id。
#[must_use]
pub fn confirmation_id(item_id: &str) -> Option<&str> {
    item_id.strip_prefix("c-")
}

#[cfg(test)]
mod tests {
    use super::*;
    use wonton_protocol::{CardChoice, ConfirmationState};

    fn card() -> ConfirmationCard {
        ConfirmationCard {
            id: "abc".into(),
            state: ConfirmationState::Pending,
            title: "安装 Immich？".into(),
            lines: Vec::new(),
            choices: vec![
                CardChoice {
                    answer: Answer::Approve,
                    label: "允许".into(),
                    choice_id: None,
                },
                CardChoice {
                    answer: Answer::Reject,
                    label: "拒绝".into(),
                    choice_id: None,
                },
            ],
            expires_at_ms: 2,
            window: None,
            plugin_id: None,
            secret_inputs: Vec::new(),
            settings: None,
            requester: None,
            category: None,
            origin: None,
            audience: CardAudience::Keeper,
            answered_by: None,
            permissions: Vec::new(),
            devices: Vec::new(),
        }
    }

    #[test]
    fn a_pending_card_becomes_a_pending_confirm() {
        let item = project_confirmation(&card(), Some("owner"), 1);
        assert_eq!(item.id, "c-abc");
        assert_eq!(item.kind, InboxType::Confirm);
        assert_eq!(item.status, InboxStatus::Pending);
        assert!(matches!(item.recipient, InboxRecipient::Person { .. }));
        let InboxPayload::Confirm(payload) = &item.payload else {
            panic!("不是 confirm")
        };
        assert_eq!(payload.choices.len(), 2);
    }

    #[test]
    fn a_rejected_card_is_declined_and_read_only() {
        let mut card = card();
        card.state = ConfirmationState::Rejected;
        card.answered_by = Some("owner".into());
        let item = project_confirmation(&card, Some("owner"), 1);
        assert_eq!(item.status, InboxStatus::Handled);
        assert_eq!(item.outcome, Some(InboxOutcome::Declined));
        assert_eq!(confirmation_id(&item.id), Some("abc"));
    }

    #[test]
    fn admins_cards_are_for_admins() {
        let mut card = card();
        card.audience = CardAudience::Admins;
        let item = project_confirmation(&card, Some("owner"), 1);
        assert!(matches!(item.recipient, InboxRecipient::Admins));
    }
}
