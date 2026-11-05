use std::collections::HashSet;

use wonton_matrix::{Message, MessageBody, UserId};

#[derive(Clone, Debug)]
pub struct IncomingText {
    pub text: String,
}

pub fn new_owner_messages(
    messages: &[Message],
    owner: &UserId,
    seen: &mut HashSet<String>,
) -> Vec<IncomingText> {
    messages
        .iter()
        .filter(|message| !message.is_own && message.sender == owner)
        .filter_map(|message| {
            let event_id = message.event_id.as_ref()?.to_string();
            if !seen.insert(event_id) {
                return None;
            }
            let MessageBody::Text(text) = &message.body else {
                return Some(IncomingText {
                    text: String::new(),
                });
            };
            Some(IncomingText { text: text.clone() })
        })
        .collect()
}
