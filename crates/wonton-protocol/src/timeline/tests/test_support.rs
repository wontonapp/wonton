//! 测试夹具：条目样例、图片和文件样例、确认卡片、消息构造和 JSON 往返。

use serde::Serialize;
use serde::de::DeserializeOwned;

use super::super::{
    FileAttachment, ImageCaption, ImageContent, MediaHandle, MessageAction, MessageContent,
    MessageReceipt, RichBlock, RichBlockKind, RichLink, RichLinkKind, RichSpan, TimelineContent,
    TimelineItem, TimelineMessage,
};
use crate::{
    AgentRunChoice, AgentRunState, Answer, CardChoice, CardLine, ChatFailure, ChatProblem,
    ConfirmationCard, ConfirmationState, ConfirmationView, FileId, SendState,
};

/// 条目列表的样例，从下面的往返测试里挪出来，函数保持在门禁行数内。
pub(super) fn sample_items() -> Vec<TimelineItem> {
    let media = MediaHandle {
        token: "mxc://home.example/abc#key".into(),
    };
    vec![
        item("1", TimelineContent::TimelineStart),
        item(
            "2",
            TimelineContent::DateDivider {
                timestamp_ms: 1_793_000_000_000,
            },
        ),
        item(
            "3",
            message(
                sample_image(media.clone()),
                SendState::Sending {
                    progress: Some(0.5),
                },
            ),
        ),
        item("3a", message(sample_file(media.clone()), SendState::Sent)),
        item(
            "4",
            message(
                MessageContent::Undecryptable,
                SendState::Sending { progress: None },
            ),
        ),
        item(
            "4a",
            message(confirmation(Some(Answer::Approve), None), SendState::Sent),
        ),
        item(
            "4b",
            message(
                confirmation(
                    None,
                    Some(ChatProblem {
                        reason: ChatFailure::TransportUnavailable,
                        message: "连不上".into(),
                    }),
                ),
                SendState::Sent,
            ),
        ),
        item(
            "4c",
            TimelineContent::AgentRun {
                run: Some("$request".into()),
                state: AgentRunState::Waiting,
                step: Some("执行命令".into()),
                steps: Some(3),
                progress: Some(0.5),
                result: None,
                end: None,
                body: "等你确认".into(),
                card_id: Some("card-1".into()),
                choices: vec![AgentRunChoice {
                    key: "allow".into(),
                    label: "继续".into(),
                    chosen: true,
                    send: crate::ChoiceSend::Sending,
                }],
                sender_name: "Hermes".into(),
                timestamp_ms: 1_793_000_000_000,
                unread_anchor: false,
            },
        ),
        item("5", TimelineContent::ReadMarker),
        item(
            "6",
            TimelineContent::Notice {
                text: "Hermes 离开了房间".into(),
            },
        ),
    ]
}

/// 图片样例：有缩略图和说明。
pub(super) fn sample_image(media: MediaHandle) -> MessageContent {
    MessageContent::Image {
        image: ImageContent {
            thumbnail: Some(media.clone()),
            filename: "cat.jpg".into(),
            mime_type: Some("image/jpeg".into()),
            size: Some(4096),
            width: Some(640),
            height: Some(480),
            caption: Some(ImageCaption {
                body: "哪张好看".into(),
                blocks: Vec::new(),
            }),
            media,
        },
    }
}

/// 文件样例：带 MIME、大小和说明。
pub(super) fn sample_file(media: MediaHandle) -> MessageContent {
    MessageContent::File {
        file: FileAttachment {
            filename: "2025-报税表.pdf".into(),
            mime_type: Some("application/pdf".into()),
            size: Some(262_144),
            caption: Some(ImageCaption {
                body: "上个月的账单".into(),
                blocks: Vec::new(),
            }),
            media,
        },
    }
}

pub(super) fn text_with_file() -> MessageContent {
    MessageContent::Text {
        body: "**你好**".into(),
        blocks: vec![RichBlock {
            kind: RichBlockKind::ListItem {
                marker: "•".into()
            },
            depth: 1,
            quoted: true,
            spans: vec![RichSpan {
                text: "你好".into(),
                bold: true,
                link: Some(RichLink {
                    url: "https://example.com/".into(),
                    kind: RichLinkKind::External,
                }),
                ..RichSpan::default()
            }],
        }],
        files: vec![FileId::from_bytes([7; 16])],
    }
}

pub(super) fn confirmation(
    sending: Option<Answer>,
    problem: Option<ChatProblem>,
) -> MessageContent {
    MessageContent::Confirmation {
        view: ConfirmationView {
            card: ConfirmationCard {
                secret_inputs: Vec::new(),
                settings: None,
                permissions: Vec::new(),
                devices: Vec::new(),
                requester: None,
                origin: None,
                audience: crate::CardAudience::Keeper,
                answered_by: None,
                category: None,
                id: "abc".into(),
                state: ConfirmationState::Pending,
                title: "Hermes 想安装 Grafana".into(),
                lines: vec![CardLine {
                    text: "能访问 Docker".into(),
                    emphasis: true,
                }],
                choices: vec![CardChoice {
                    answer: Answer::Approve,
                    label: "安装".into(),
                    choice_id: None,
                }],
                expires_at_ms: 1_793_000_600_000,
                plugin_id: None,
                window: None,
            },
            sending,
            sending_choice_id: None,
            problem,
            window: None,
        },
    }
}

pub(super) fn round_trip<T: Serialize + DeserializeOwned>(value: &T) -> T {
    serde_json::from_str(&serde_json::to_string(value).unwrap()).unwrap()
}

pub(super) fn message(body: MessageContent, send_state: SendState) -> TimelineContent {
    TimelineContent::Message {
        message: TimelineMessage {
            mentions_me: false,
            unread_anchor: false,
            event_id: Some("$event".into()),
            sender_id: "@hermes:home.example".into(),
            sender_name: "Hermes".into(),
            is_own: false,
            timestamp_ms: 1_793_000_000_000,
            send_state,
            body,
            edited: true,
            read_by: vec!["主人".into()],
            reply: None,
            side_chat: None,
            preview: None,
            actions: vec![MessageAction::Copy {
                text: "好的".into(),
            }],
            group: Some(crate::MessageGroup {
                id: "3f0c9a1e5b7d42c6a8e1f0b2c4d6e8a0".into(),
                index: 0,
                count: 2,
            }),
            receipt: Some(MessageReceipt::Read),
            starts_time_group: true,
            run: crate::MessageRun::Last,
            status: Some(crate::MessageStatus::Read),
            choices: Some(crate::MessageChoices {
                options: vec![crate::MessageChoice {
                    key: "✅".into(),
                    label: Some("批准一次".into()),
                    chosen: true,
                    send: crate::ChoiceSend::Failed {
                        message: "连不上".into(),
                    },
                }],
                state: crate::ChoicesState::Answered,
            }),
            sender_avatar: Some(MediaHandle {
                token: "avatar".into(),
            }),
        },
    }
}

pub(super) fn item(id: &str, content: TimelineContent) -> TimelineItem {
    TimelineItem {
        id: id.into(),
        content,
    }
}
