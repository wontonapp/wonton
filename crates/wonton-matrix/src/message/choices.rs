//! 消息的选项（ADR-0109 第 8 节）：发送者自己先放的表情就是选项。

use wonton_protocol::{ChoiceSend, ChoicesState, MessageChoice, MessageChoices};

use super::content::MessageBody;
use super::item::{Message, OwnReaction};

impl Message {
    /// 这条消息的选项（ADR-0109 第 8 节）：发送者自己先放的表情就是选项，核心不认表情的含义。发送者选完会撤掉
    /// 自己放的表情，这时正文里还有对应行的、我加过的表情照样算。正在发和发失败的不算选上。
    ///
    /// 自己的消息、没有选项时是 `None`。选项按 SDK 给的回应顺序，没有谁先谁后的记录。
    pub fn choices(&self) -> Option<MessageChoices> {
        if self.is_own {
            return None;
        }
        let options: Vec<MessageChoice> = self
            .reactions
            .iter()
            .filter_map(|reaction| {
                let label = self.label(&reaction.key);
                let chosen = matches!(reaction.own, Some(OwnReaction::Sending | OwnReaction::Sent));
                let from_sender = reaction.senders.contains(&self.sender);
                (from_sender || (reaction.own.is_some() && label.is_some())).then(|| {
                    MessageChoice {
                        key: reaction.key.clone(),
                        label,
                        chosen,
                        send: match &reaction.own {
                            None | Some(OwnReaction::Sent) => ChoiceSend::Idle,
                            Some(OwnReaction::Sending) => ChoiceSend::Sending,
                            Some(OwnReaction::Failed { error, .. }) => ChoiceSend::Failed {
                                message: error.clone(),
                            },
                        },
                    }
                })
            })
            .collect();
        if options.is_empty() {
            return None;
        }
        let state = if options.iter().any(|option| option.chosen) {
            ChoicesState::Answered
        } else if self.is_direct || self.mentions_me {
            ChoicesState::Open
        } else {
            ChoicesState::NotForMe
        };
        Some(MessageChoices { options, state })
    }

    /// 按钮文字：正文里以 `key` 开头的第一行，去掉表情和后面的 `=`、`-`、`—`、`:` 和空白。没有这样一行或者
    /// 去掉之后是空的就只画表情（`None`）。
    fn label(&self, key: &str) -> Option<String> {
        let body = match &self.body {
            MessageBody::Text(body) | MessageBody::Notice(body) => body,
            _ => return None,
        };
        let line = body
            .lines()
            .find(|line| line.trim_start().starts_with(key))?;
        let rest = line
            .trim_start()
            .strip_prefix(key)?
            .trim_start_matches(|c: char| {
                c.is_whitespace() || matches!(c, '=' | '-' | '—' | ':' | '：')
            });
        let label = rest.trim();
        (!label.is_empty()).then(|| label.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::super::item::{Reaction, SendStatus};
    use serde_json::Map;

    use super::*;

    fn message(body: &str, reactions: &[(&str, Option<OwnReaction>)]) -> Message {
        Message {
            event_id: Some("$1".parse().unwrap()),
            sender: "@bot:s".parse().unwrap(),
            sender_name: "Bot".to_owned(),
            is_own: false,
            timestamp_ms: 0,
            status: SendStatus::Sent,
            body: MessageBody::Notice(body.to_owned()),
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
            reactions: reactions
                .iter()
                .map(|(key, own)| Reaction {
                    key: (*key).to_owned(),
                    senders: vec!["@bot:s".parse().unwrap()],
                    own: own.clone(),
                })
                .collect(),
            is_direct: true,
            mentions_me: false,
            sender_avatar: None,
        }
    }

    #[test]
    fn the_label_is_the_first_line_starting_with_the_key() {
        let body = "批一下\n\n✅ = 批准一次\n❌ - deny\n❎: 拒绝\n✅—go";
        let parsed = message(body, &[]);
        assert_eq!(parsed.label("✅").as_deref(), Some("批准一次"));
        assert_eq!(parsed.label("❌").as_deref(), Some("deny"));
        assert_eq!(parsed.label("❎").as_deref(), Some("拒绝"));
        assert_eq!(parsed.label("✅—").as_deref(), Some("go"));
        assert_eq!(message("✅—go", &[]).label("✅").as_deref(), Some("go"));
        assert_eq!(message("同意 ✅ 继续", &[]).label("✅"), None);
        assert_eq!(message("✅   ", &[]).label("✅"), None);
        assert_eq!(message("没有表情", &[]).label("✅"), None);
    }

    #[test]
    fn the_sender_reacting_makes_an_option_and_choosing_answers_the_message() {
        let choices = message(
            "✅ = approve\n❌ = deny",
            &[("✅", Some(OwnReaction::Sent)), ("❌", None), ("👍", None)],
        )
        .choices()
        .expect("有选项");
        assert_eq!(
            choices
                .options
                .iter()
                .map(|option| (option.key.as_str(), option.label.as_deref()))
                .collect::<Vec<_>>(),
            [("✅", Some("approve")), ("❌", Some("deny")), ("👍", None)]
        );
        assert_eq!(choices.state, ChoicesState::Answered);
        assert_eq!(choices.options[0].send, ChoiceSend::Idle);
    }

    #[test]
    fn a_failed_reaction_is_open_and_carries_the_reason() {
        let choices = message(
            "✅ = approve",
            &[(
                "✅",
                Some(OwnReaction::Failed {
                    error: "连不上".to_owned(),
                    recoverable: true,
                }),
            )],
        )
        .choices()
        .expect("有选项");
        assert_eq!(choices.state, ChoicesState::Open);
        assert_eq!(
            choices.options[0].send,
            ChoiceSend::Failed {
                message: "连不上".to_owned()
            }
        );
    }

    #[test]
    fn a_key_only_i_reacted_to_needs_the_body_line_and_only_someone_else_can_answer() {
        // Bot 选完撤掉了自己的表情：我加过、正文里还有那一行的照样是选项。
        let mine = |key: &str| Reaction {
            key: key.to_owned(),
            senders: vec!["@owner:s".parse().unwrap()],
            own: Some(OwnReaction::Sent),
        };
        let mut answered = message("✅ = approve", &[]);
        answered.reactions = vec![mine("✅")];
        assert_eq!(
            answered.choices().expect("有选项").state,
            ChoicesState::Answered
        );

        let mut stray = message("随便说一句", &[]);
        stray.reactions = vec![mine("✅")];
        assert_eq!(stray.choices(), None);

        // 群里没提到我：看得见，点不了。
        let mut group = message("✅ = approve", &[("✅", None)]);
        group.is_direct = false;
        assert_eq!(
            group.choices().expect("有选项").state,
            ChoicesState::NotForMe
        );
        group.mentions_me = true;
        assert_eq!(group.choices().expect("有选项").state, ChoicesState::Open);

        let mut own = message("✅ = approve", &[("✅", None)]);
        own.is_own = true;
        assert_eq!(own.choices(), None);
    }
}
