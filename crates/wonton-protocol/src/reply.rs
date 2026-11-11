//! 回复：消息上方引用的原消息、回复菜单项带的引用、输入框草稿里的回复对象（ADR-0073）。
//!
//! 引用的文字和类别都由核心给，壳按类别画（图片画缩略图，撤回、解不开的按类别写固定文字），不自己摘要原消息。

use serde::{Deserialize, Serialize};

use crate::MediaHandle;

/// 被引用的那条消息。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct QuotedMessage {
    /// 原消息的事件 ID，回复时原样交回核心（[`TimelineAction::SendText`](crate::TimelineAction::SendText) 的 `reply_to`）。
    pub event_id: String,
    pub sender_id: String,
    /// 发送者的显示名，没有设置时是用户 ID。
    pub sender_name: String,
    pub content: QuotedContent,
}

/// 被引用消息的内容，壳画成一两行的摘要。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum QuotedContent {
    /// 去掉格式的纯文字，核心已截到摘要够用的长度，壳再按版面截断。
    Text {
        text: String,
    },
    Image {
        thumbnail: Option<MediaHandle>,
    },
    /// 原消息已撤回。
    Redacted,
    /// 原消息是加密的，本设备还没有密钥。
    Undecryptable,
    /// 确认卡片、还不支持显示的类型等，壳写“一条消息”之类的固定文字。
    Other,
}

/// 消息上方引用的原消息。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ReplyQuote {
    /// 原消息不在本地，核心正在向服务器取，取到后换成 [`ReplyQuote::Ready`]。
    Loading {
        event_id: String,
    },
    Ready {
        message: QuotedMessage,
    },
    /// 取不到：服务器上找不到、没有权限看，或者取的时候出错。核心不自动重取。
    Unavailable {
        event_id: String,
    },
}

/// 一个房间输入框的草稿，`ClientCore::draft` 返回它；用 [`TimelineAction::SetDraft`](crate::TimelineAction::SetDraft) 记下。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ComposerDraft {
    pub body: String,
    /// 正在回复的消息；壳在输入框上方画出来，发送时把它的事件 ID 作为 `reply_to` 交回。
    pub reply: Option<ReplyQuote>,
    /// 输入框里第一个外部链接的预览（ADR-0065）；没有链接、刚被去掉（[`Action::DismissLinkPreview`](crate::Action::DismissLinkPreview)）或还没取到时是 `None`。
    pub preview: Option<crate::LinkPreviewState>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_round_trip_through_json() {
        let quotes = [
            ReplyQuote::Loading {
                event_id: "$a".into(),
            },
            ReplyQuote::Ready {
                message: QuotedMessage {
                    event_id: "$b".into(),
                    sender_id: "@hermes:home.example".into(),
                    sender_name: "Hermes".into(),
                    content: QuotedContent::Image { thumbnail: None },
                },
            },
            ReplyQuote::Unavailable {
                event_id: "$c".into(),
            },
        ];
        for quote in quotes {
            let draft = ComposerDraft {
                body: "ok".into(),
                reply: Some(quote),
                preview: None,
            };
            let json = serde_json::to_string(&draft).unwrap();
            assert_eq!(serde_json::from_str::<ComposerDraft>(&json).unwrap(), draft);
        }
    }
}
