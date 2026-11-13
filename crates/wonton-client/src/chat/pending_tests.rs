use super::{LONG_WAIT_MS, Witnessed, derive};
use crate::chat::confirmation::Answers;
use wonton_protocol::{ConfirmationList, PendingConfirmation};

fn pending(id: &str, bot: bool, now: u64) -> wonton_protocol::ConfirmationCard {
    serde_json::from_value(serde_json::json!({
        "id": id,
        "state": {"type": "pending"},
        "title": id,
        "lines": [],
        "choices": [{"answer": "reject", "label": "Deny"}],
        "expires_at_ms": now + 600_000,
        "requester": if bot {
            serde_json::json!({"type":"bot", "bot":{"plugin_id":"agent.test", "bot":"test"}, "user_id":"@bot:example.com"})
        } else {
            serde_json::Value::Null
        },
    })).unwrap()
}

fn ongoing(cards: &[PendingConfirmation]) -> Vec<&str> {
    cards
        .iter()
        .filter(|card| card.ongoing)
        .map(|card| card.id.as_str())
        .collect()
}

#[test]
fn only_continuously_waiting_bot_cards_become_ongoing_and_ending_clears_them() {
    let start = 1_000_000;
    let bot = pending("bot", true, start);
    let short = pending("short", false, start);
    let mut list = ConfirmationList {
        revision: 1,
        confirmations: vec![bot.clone(), short],
        finished: Vec::new(),
        contact_requests: Vec::new(),
    };
    let answers = Answers::default();
    let mut seen = Witnessed::default();
    assert!(ongoing(&derive(Some(&list), &answers, start, &mut seen).chat_pending).is_empty());
    assert!(
        ongoing(&derive(Some(&list), &answers, start + LONG_WAIT_MS - 1, &mut seen).chat_pending)
            .is_empty()
    );
    let ready = derive(Some(&list), &answers, start + LONG_WAIT_MS, &mut seen);
    assert_eq!(ongoing(&ready.chat_pending), ["bot"]);
    list.confirmations.retain(|card| card.id != "bot");
    list.finished.push(wonton_protocol::ConfirmationCard {
        state: wonton_protocol::ConfirmationState::Rejected,
        ..bot
    });
    let ended = derive(Some(&list), &answers, start + LONG_WAIT_MS + 1, &mut seen);
    assert!(ongoing(&ended.chat_pending).is_empty());
    assert!(ended.chat_settled.contains(&"bot".to_owned()));
}
