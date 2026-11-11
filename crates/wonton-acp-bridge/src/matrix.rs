use std::collections::HashSet;

use wonton_matrix::{Message, MessageBody, OwnedUserId, UserId};

#[derive(Clone, Debug)]
pub struct IncomingText {
    pub text: String,
    /// 这条消息的事件 ID；撤回（如转交登录密钥后）要用它。
    pub event_id: String,
    /// 所属话题的根消息事件 ID；在主对话里发的消息是 `None`。
    pub thread_root: Option<String>,
}

pub fn new_allowed_messages(
    messages: &[Message],
    allowed_users: &[OwnedUserId],
    bot: &UserId,
    group: bool,
    seen: &mut HashSet<String>,
) -> Vec<IncomingText> {
    messages
        .iter()
        .filter(|message| !message.is_own && allowed_users.contains(&message.sender))
        .filter_map(|message| {
            // 密钥可能晚于事件到达；正文解密后的同一事件仍需交给 Agent。
            if matches!(message.body, MessageBody::Undecryptable) {
                return None;
            }
            let event_id = message.event_id.as_ref()?.to_string();
            let thread_root = message.thread_root.as_ref().map(ToString::to_string);
            if !seen.insert(event_id.clone()) {
                return None;
            }
            let MessageBody::Text(text) = &message.body else {
                return Some(IncomingText {
                    text: String::new(),
                    event_id,
                    thread_root,
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
                thread_root,
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
            agent_run: None,
            thread_root: None,
            edited: false,
            read_by: Vec::new(),
            reply: None,
            can_redact: false,
        }
    }

    #[test]
    fn delayed_decryption_delivers_the_allowed_message_once() {
        let owner: OwnedUserId = "@owner:example.org".parse().unwrap();
        let bot: OwnedUserId = "@bot:example.org".parse().unwrap();
        let allowed = [owner.clone()];
        let mut message = owner_message("$delayed:example.org", "", &[]);
        message.body = MessageBody::Undecryptable;
        let mut seen = HashSet::new();
        assert!(
            new_allowed_messages(&[message.clone()], &allowed, &bot, false, &mut seen).is_empty()
        );
        message.body = MessageBody::Text("hello".into());
        let incoming = new_allowed_messages(&[message.clone()], &allowed, &bot, false, &mut seen);
        assert_eq!(incoming[0].text, "hello");
        assert_eq!(incoming[0].event_id, "$delayed:example.org");
        assert_eq!(incoming.len(), 1);
        assert!(new_allowed_messages(&[message], &allowed, &bot, false, &mut seen).is_empty());
        // 名单外的人说话不交给 Agent。
        let mut stranger = owner_message("$stranger:example.org", "hello", &[]);
        stranger.sender = "@stranger:example.org".parse().unwrap();
        assert!(new_allowed_messages(&[stranger], &allowed, &bot, false, &mut seen).is_empty());
    }

    #[test]
    fn a_group_only_takes_commands_that_mention_the_bot() {
        let owner: OwnedUserId = "@owner:example.org".parse().unwrap();
        let bot: OwnedUserId = "@bot:example.org".parse().unwrap();
        let allowed = [owner.clone()];
        let plain = owner_message("$plain:example.org", "普通消息", &[]);
        let command = owner_message("$command:example.org", "/review", &[]);
        let mentioned = owner_message("$mentioned:example.org", "/review", &["@bot:example.org"]);
        let mut seen = HashSet::new();
        let incoming = new_allowed_messages(
            &[plain.clone(), command, mentioned],
            &allowed,
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
            new_allowed_messages(
                &[plain, owner_message("$c2:example.org", "/review", &[])],
                &allowed,
                &bot,
                false,
                &mut seen
            )
            .len(),
            2
        );
    }
}
