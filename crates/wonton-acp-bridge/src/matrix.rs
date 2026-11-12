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
            // 密钥可能晚于事件到达；正文解密后的同一事件仍需交给 Agent。
            if matches!(message.body, MessageBody::Undecryptable) {
                return None;
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use wonton_matrix::SendStatus;

    #[test]
    fn delayed_decryption_delivers_the_owner_message_once() {
        let owner = "@owner:example.org".parse().unwrap();
        let mut message = Message {
            event_id: Some("$delayed:example.org".parse().unwrap()),
            sender: owner,
            sender_name: String::new(),
            is_own: false,
            timestamp_ms: 1,
            status: SendStatus::Sent,
            body: MessageBody::Undecryptable,
            custom: Default::default(),
            edited: false,
            read_by: Vec::new(),
        };
        let owner = message.sender.clone();
        let mut seen = HashSet::new();
        assert!(new_owner_messages(&[message.clone()], &owner, &mut seen).is_empty());
        message.body = MessageBody::Text("hello".into());
        let incoming = new_owner_messages(&[message.clone()], &owner, &mut seen);
        assert_eq!(incoming[0].text, "hello");
        assert_eq!(incoming.len(), 1);
        assert!(new_owner_messages(&[message], &owner, &mut seen).is_empty());
    }
}
