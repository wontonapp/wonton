//! 打开的房间的时间线：按增量推送（ADR-0005 的验证结果）。
//!
//! 时间线可能有成百上千条，每次变化都推整份太浪费，所以推的是增量批次，不能合并：壳要按顺序把每一批应用到
//! 自己持有的列表上。第一批总是 [`TimelineDiff::Reset`]，给出当前的全部条目。条目和 matrix-rust-sdk 的时间线
//! 一一对应（含日期分隔线、已读标记等），下标就是 SDK 的下标；哪些事件进时间线由 `wonton-matrix` 的事件过滤决定。

use serde::{Deserialize, Serialize};

use crate::{ConfirmationView, FileId};

/// 一批时间线变化，以及向前翻页的状态。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct TimelineUpdate {
    /// 按顺序应用的增量；只有翻页状态变化时为空。
    pub diffs: Vec<TimelineDiff>,
    pub back_pagination: BackPagination,
}

/// 对条目列表的一次修改，语义同 `eyeball-im` 的 `VectorDiff`。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TimelineDiff {
    /// 在末尾追加多条。
    Append {
        items: Vec<TimelineItem>,
    },
    /// 清空。
    Clear,
    PushFront {
        item: TimelineItem,
    },
    PushBack {
        item: TimelineItem,
    },
    PopFront,
    PopBack,
    /// 在 `index` 处插入，原来的条目后移。
    Insert {
        index: u32,
        item: TimelineItem,
    },
    /// 替换 `index` 处的条目（同一条消息的内容、发送状态或已读者变了）。
    Set {
        index: u32,
        item: TimelineItem,
    },
    Remove {
        index: u32,
    },
    /// 只保留前 `length` 条。
    Truncate {
        length: u32,
    },
    /// 整个换成 `items`。
    Reset {
        items: Vec<TimelineItem>,
    },
}

/// 时间线里的一条。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct TimelineItem {
    /// 在这条时间线里稳定的标识，可以直接用作列表的 ID：本地回显发出后、内容被编辑后都不变。
    pub id: String,
    pub content: TimelineContent,
}

/// 条目的内容。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
#[expect(
    clippy::large_enum_variant,
    reason = "跨 FFI 时按值序列化，变体大小差别不影响；UniFFI 的记录字段不支持 Box"
)]
pub enum TimelineContent {
    Message {
        message: TimelineMessage,
    },
    /// 一行说明，比如成员加入、离开，房间改名。文字由核心给出。
    Notice {
        text: String,
    },
    /// 有人改了名字或头像（ADR-0044），文字由壳写：名字变了是“旧名 改名为 新名”，只换了头像是“某某 换了头像”。
    ProfileChange {
        user_id: String,
        /// 改之后的名字。
        name: String,
        /// 改之前的名字；名字没变时为空。
        previous_name: Option<String>,
        avatar_changed: bool,
    },
    /// 日期分隔线，`timestamp_ms` 是这一天里第一条消息的时间（Unix 毫秒），壳按本地时区显示日期。
    DateDivider {
        timestamp_ms: u64,
    },
    /// “以下是新消息”：本账号的已读位置。
    ReadMarker,
    /// 已经翻到房间的开头。
    TimelineStart,
}

/// 一条消息。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct TimelineMessage {
    /// 服务器分配的事件 ID；本地回显还没发出去时为空。
    pub event_id: Option<String>,
    /// 发送者的 Matrix 用户 ID。
    pub sender_id: String,
    /// 发送者的显示名，没有设置时是用户 ID。
    pub sender_name: String,
    /// 是否由本账号发出（包括本账号的其他设备）。
    pub is_own: bool,
    /// 发送时间，Unix 毫秒。
    pub timestamp_ms: u64,
    pub send_state: SendState,
    pub body: MessageContent,
    /// 是否被编辑过（显示的是最新内容）。Bot 的流式回复靠编辑消息实现。
    pub edited: bool,
    /// 读过这条的其他人（显示名）。只有自己发的、别人读到的最后一条才有，壳有就显示“已读”。
    pub read_by: Vec<String>,
    /// 长按这条消息能做的事。空的就没有菜单。
    pub actions: Vec<MessageAction>,
}

/// 长按消息的菜单里的一项。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MessageAction {
    /// 把 `text` 放进剪贴板。
    Copy { text: String },
}

/// 发送状态。别人的消息和从服务器同步来的消息都是 [`SendState::Sent`]。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SendState {
    /// 本地回显，在排队或正在发送。排在失败消息后面时，直到那条发出或取消才继续。
    /// `progress` 是核心合并原图和缩略图后算好的上传比例，范围 `0.0..=1.0`；`None` 表示不确定（文字、排队或总量未知）。
    /// 重试、重发或重启后可能倒退或回到 `None`，壳原样绘制，不自行换算或保持单调。
    /// `Some(1.0)` 不代表发送成功，成功和失败分别由 [`SendState::Sent`] 和 [`SendState::Failed`] 表示。
    Sending {
        progress: Option<f32>,
    },
    Sent,
    /// 发送失败，`message` 是原因。没发出去的都可以取消（[`Action::CancelSend`](crate::Action::CancelSend)）。
    ///
    /// - `recoverable` 为真：连接出了问题（连不上、超时、服务器暂时出错）。核心隔一段时间自动重发，连续失败时间隔
    ///   逐渐拉长，直到发出去；也可以马上重发（[`Action::RetrySend`](crate::Action::RetrySend)）。
    /// - 为假：服务器拒收（比如太大、没有权限）。核心不自动重发，同一房间之后发的消息都排在它后面，直到主人重发或者
    ///   取消这一条。
    Failed {
        message: String,
        recoverable: bool,
    },
}

/// 消息内容。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MessageContent {
    /// 文字，包括 Bot 常用的 `m.notice`。`body` 是去掉文件链接后的原文，消息只有链接时为空；`blocks` 是它排好版的样子，
    /// 壳画这个，不解析 Markdown；`files` 是认出的文件，壳在消息下面按编号画卡片（ADR-0020）。
    Text {
        body: String,
        blocks: Vec<RichBlock>,
        files: Vec<FileId>,
    },
    Image {
        image: ImageContent,
    },
    /// 确认卡片（ADR-0016）。只有核心账号发的才是卡片，别人发的同样结构的内容是 [`MessageContent::Text`]。卡片靠编辑
    /// 消息更新状态，[`TimelineMessage::edited`] 总是假。
    Confirmation {
        view: ConfirmationView,
    },
    /// 端到端加密的消息，本设备还没有解密密钥；拿到密钥后核心会把它换成解密后的内容。
    Undecryptable,
    /// 已撤回。
    Redacted,
    /// 还不支持显示的类型（文件、语音、投票等），`kind` 是消息或事件类型。
    Unsupported {
        kind: String,
    },
}

/// 排好版的一块文字。没有嵌套：列表和引用的层次摊平成每一块的 `depth` 和 `quoted`。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct RichBlock {
    pub kind: RichBlockKind,
    /// 缩进几层。
    pub depth: u32,
    /// 在引用里。
    pub quoted: bool,
    pub spans: Vec<RichSpan>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RichBlockKind {
    Paragraph,
    Heading,
    /// `marker` 是项目符号、序号或任务框，壳原样画在文字前面。
    ListItem {
        marker: String,
    },
    /// 代码块：等宽字体，不折行，横向滚动。
    Code,
    /// 表格，第一行是表头，每行格数相同。不折行，横向滚动。
    Table {
        rows: Vec<Vec<String>>,
    },
    /// 分隔线，`spans` 为空。
    Rule,
}

/// 一段样式相同的文字。`link` 只会是 http、https 或 mailto 的地址。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct RichSpan {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub code: bool,
    pub strike: bool,
    pub link: Option<String>,
}

/// 图片消息。原图和缩略图都经 `ClientCore::load_media` 取，核心负责下载、解密和缓存。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ImageContent {
    /// 原图。
    pub media: MediaHandle,
    /// 发送端给的缩略图；时间线里先显示它，点开再取原图。没有时为空。
    pub thumbnail: Option<MediaHandle>,
    pub filename: String,
    /// MIME 类型，发送方没有声明时为空。
    pub mime_type: Option<String>,
    /// 字节数。
    pub size: Option<u64>,
    /// 像素宽度，壳据此预留版面。
    pub width: Option<u64>,
    pub height: Option<u64>,
}

/// 取媒体用的句柄。对壳是不透明的，原样交回核心即可；里面可能有加密附件的密钥，不要写进日志。
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct MediaHandle {
    pub token: String,
}

impl std::fmt::Debug for MediaHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MediaHandle(<redacted>)")
    }
}

/// 向前翻页（加载更早的消息）的状态。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum BackPagination {
    /// 没在翻页，还有更早的消息。
    Idle,
    Paginating,
    /// 已经到了房间的开头。
    ReachedStart,
}

#[cfg(test)]
mod tests {
    use serde::de::DeserializeOwned;

    use super::*;
    use crate::{
        Answer, CardChoice, CardLine, ChatFailure, ChatProblem, ConfirmationCard, ConfirmationState,
    };

    /// 契约：时间线增量的每种变化和每种条目都能经 JSON 往返；媒体句柄不进 Debug 输出。
    #[test]
    fn timeline_updates_round_trip_through_json() {
        let media = MediaHandle {
            token: "mxc://home.example/abc#key".into(),
        };
        let items = vec![
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
                    MessageContent::Image {
                        image: ImageContent {
                            media: media.clone(),
                            thumbnail: Some(media.clone()),
                            filename: "cat.jpg".into(),
                            mime_type: Some("image/jpeg".into()),
                            size: Some(4096),
                            width: Some(640),
                            height: Some(480),
                        },
                    },
                    SendState::Sending {
                        progress: Some(0.5),
                    },
                ),
            ),
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
            item("5", TimelineContent::ReadMarker),
            item(
                "6",
                TimelineContent::Notice {
                    text: "Hermes 离开了房间".into(),
                },
            ),
        ];
        let update = TimelineUpdate {
            diffs: vec![
                TimelineDiff::Reset {
                    items: items.clone(),
                },
                TimelineDiff::Append { items },
                TimelineDiff::Set {
                    index: 2,
                    item: item(
                        "3",
                        message(
                            text_with_file(),
                            SendState::Failed {
                                message: "网络断开".into(),
                                recoverable: true,
                            },
                        ),
                    ),
                },
                TimelineDiff::Insert {
                    index: 0,
                    item: item("0", message(MessageContent::Redacted, SendState::Sent)),
                },
                TimelineDiff::Remove { index: 1 },
                TimelineDiff::Truncate { length: 3 },
                TimelineDiff::PopFront,
                TimelineDiff::Clear,
            ],
            back_pagination: BackPagination::ReachedStart,
        };
        assert_eq!(round_trip(&update), update);
        assert!(
            !format!("{update:?}").contains("mxc://"),
            "Debug 不能输出媒体句柄"
        );
    }

    fn text_with_file() -> MessageContent {
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
                    link: Some("https://example.com/".into()),
                    ..RichSpan::default()
                }],
            }],
            files: vec![FileId::from_bytes([7; 16])],
        }
    }

    fn confirmation(sending: Option<Answer>, problem: Option<ChatProblem>) -> MessageContent {
        MessageContent::Confirmation {
            view: ConfirmationView {
                card: ConfirmationCard {
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

    #[test]
    fn sending_progress_has_a_stable_json_contract() {
        for (state, json) in [
            (
                SendState::Sending {
                    progress: Some(0.5),
                },
                serde_json::json!({"type": "sending", "progress": 0.5}),
            ),
            (
                SendState::Sending { progress: None },
                serde_json::json!({"type": "sending", "progress": null}),
            ),
        ] {
            assert_eq!(serde_json::to_value(&state).unwrap(), json);
            assert_eq!(serde_json::from_value::<SendState>(json).unwrap(), state);
        }
        let state: SendState = serde_json::from_str(r#"{"type":"sending"}"#).unwrap();
        assert_eq!(state, SendState::Sending { progress: None });
    }

    fn round_trip<T: Serialize + DeserializeOwned>(value: &T) -> T {
        serde_json::from_str(&serde_json::to_string(value).unwrap()).unwrap()
    }

    fn message(body: MessageContent, send_state: SendState) -> TimelineContent {
        TimelineContent::Message {
            message: TimelineMessage {
                event_id: Some("$event".into()),
                sender_id: "@hermes:home.example".into(),
                sender_name: "Hermes".into(),
                is_own: false,
                timestamp_ms: 1_793_000_000_000,
                send_state,
                body,
                edited: true,
                read_by: vec!["主人".into()],
                actions: vec![MessageAction::Copy {
                    text: "好的".into(),
                }],
            },
        }
    }

    fn item(id: &str, content: TimelineContent) -> TimelineItem {
        TimelineItem {
            id: id.into(),
            content,
        }
    }
}
