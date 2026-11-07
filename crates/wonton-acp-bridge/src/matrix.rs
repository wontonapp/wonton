use std::collections::HashSet;

use wonton_matrix::{Message, MessageBody, UserId};

#[derive(Clone, Debug)]
pub struct IncomingText {
    pub text: String,
    /// 这条消息的事件 ID；撤回（如转交登录密钥后）要用它。
    pub event_id: String,
}

pub fn new_owner_messages(
    messages: &[Message],
    owner: &UserId,
    bot: &UserId,
    group: bool,
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
            if !seen.insert(event_id.clone()) {
                return None;
            }
            let MessageBody::Text(text) = &message.body else {
                return Some(IncomingText {
                    text: String::new(),
                    event_id,
                });
            };
            // 群里有别的 Bot，命令要由主人点名本 Bot；私聊不用。
            if group
                && text.trim_start().starts_with('/')
                && !message.mentions.iter().any(|mentioned| mentioned == bot)
            {
                return None;
            }
            Some(IncomingText {
                text: text.clone(),
                event_id,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use wonton_matrix::{Message, OwnedUserId, SendStatus};

    fn owner_message(event_id: &str, text: &str, mentions: &[&str]) -> Message {
        Message {
            event_id: Some(event_id.parse().unwrap()),
            sender: "@owner:example.org".parse().unwrap(),
            sender_name: String::new(),
            is_own: false,
            timestamp_ms: 1,
            status: SendStatus::Sent,
            body: MessageBody::Text(text.into()),
            custom: Default::default(),
            mentions: mentions.iter().map(|user| user.parse().unwrap()).collect(),
            thread_root: None,
            edited: false,
            read_by: Vec::new(),
        }
    }

    #[test]
    fn delayed_decryption_delivers_the_owner_message_once() {
        let owner: OwnedUserId = "@owner:example.org".parse().unwrap();
        let bot: OwnedUserId = "@bot:example.org".parse().unwrap();
        let mut message = owner_message("$delayed:example.org", "", &[]);
        message.body = MessageBody::Undecryptable;
        let mut seen = HashSet::new();
        assert!(new_owner_messages(&[message.clone()], &owner, &bot, false, &mut seen).is_empty());
        message.body = MessageBody::Text("hello".into());
        let incoming = new_owner_messages(&[message.clone()], &owner, &bot, false, &mut seen);
        assert_eq!(incoming[0].text, "hello");
        assert_eq!(incoming[0].event_id, "$delayed:example.org");
        assert_eq!(incoming.len(), 1);
        assert!(new_owner_messages(&[message], &owner, &bot, false, &mut seen).is_empty());
    }

    #[test]
    fn a_group_only_takes_commands_that_mention_the_bot() {
        let owner: OwnedUserId = "@owner:example.org".parse().unwrap();
        let bot: OwnedUserId = "@bot:example.org".parse().unwrap();
        let plain = owner_message("$plain:example.org", "普通消息", &[]);
        let command = owner_message("$command:example.org", "/review", &[]);
        let mentioned = owner_message("$mentioned:example.org", "/review", &["@bot:example.org"]);
        let mut seen = HashSet::new();
        let incoming = new_owner_messages(
            &[plain.clone(), command, mentioned],
            &owner,
            &bot,
            true,
            &mut seen,
        );
        assert_eq!(
            incoming
                .iter()
                .map(|incoming| incoming.event_id.as_str())
                .collect::<Vec<_>>(),
            ["$plain:example.org", "$mentioned:example.org"]
        );

        // 私聊不分提及。
        let mut seen = HashSet::new();
        assert_eq!(
            new_owner_messages(
                &[plain, owner_message("$c2:example.org", "/review", &[])],
                &owner,
                &bot,
                false,
                &mut seen
            )
            .len(),
            2
        );
    }
}
