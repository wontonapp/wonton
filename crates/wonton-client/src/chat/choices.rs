//! 消息下面的选项按钮（ADR-0109 第 8 节）：哪些是选项由 matrix 从发送者自己放的表情算，这里只把发送失败的
//! 原因换成人话。

use wonton_l10n::l10n;
use wonton_matrix::{Message, OwnReaction};
use wonton_protocol::{ChoiceSend, MessageChoices};

use crate::l10n::{CATALOG, language};

/// `message` 的选项；自己的消息、没有选项时是 `None`。
pub(crate) fn choices(message: &Message) -> Option<MessageChoices> {
    let mut choices = message.choices()?;
    for option in &mut choices.options {
        let ChoiceSend::Failed { message: failure } = &mut option.send else {
            continue;
        };
        let recoverable = message
            .reactions
            .iter()
            .find(|reaction| reaction.key == option.key)
            .and_then(|reaction| match &reaction.own {
                Some(OwnReaction::Failed { recoverable, .. }) => Some(*recoverable),
                _ => None,
            });
        let Some(recoverable) = recoverable else {
            continue;
        };
        let id = if recoverable {
            "chat-choice-send-failed"
        } else {
            "chat-choice-send-failed-rejected"
        };
        *failure = l10n!(CATALOG, language(), id, error = failure.clone());
    }
    Some(choices)
}

#[cfg(test)]
mod tests {
    use serde_json::Map;
    use wonton_matrix::{MessageBody, Reaction, SendStatus};

    use super::*;

    fn failed(recoverable: bool) -> MessageChoices {
        let message = Message {
            event_id: Some("$1".parse().unwrap()),
            sender: "@bot:s".parse().unwrap(),
            sender_name: "Bot".to_owned(),
            is_own: false,
            timestamp_ms: 0,
            status: SendStatus::Sent,
            body: MessageBody::Notice("✅ = approve".to_owned()),
            custom: Map::new(),
            mentions: Vec::new(),
            thread_root: None,
            thread_replies: None,
            edited: false,
            read_by: Vec::new(),
            reply: None,
            group: None,
            agent_run: None,
            can_redact: false,
            reactions: vec![Reaction {
                key: "✅".to_owned(),
                senders: vec!["@bot:s".parse().unwrap()],
                own: Some(OwnReaction::Failed {
                    error: "connect timed out".to_owned(),
                    recoverable,
                }),
            }],
            is_direct: true,
            mentions_me: false,
            sender_avatar: None,
        };
        choices(&message).expect("有选项")
    }

    fn reason(choices: MessageChoices) -> String {
        match &choices.options[0].send {
            ChoiceSend::Failed { message } => message.clone(),
            other => panic!("不是失败：{other:?}"),
        }
    }

    #[test]
    fn the_failure_reason_tells_the_two_kinds_apart_and_keeps_the_original() {
        crate::l10n::set_languages(&["zh-Hans".to_owned()]);
        let recoverable = reason(failed(true));
        let rejected = reason(failed(false));
        assert_ne!(recoverable, rejected, "连不上和被拒收要说成两回事");
        for text in [&recoverable, &rejected] {
            assert!(text.contains("connect timed out"), "{text}");
        }
    }
}
