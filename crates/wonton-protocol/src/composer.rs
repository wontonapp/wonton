//! 输入框的附件区，和点了发送、还没交给 Matrix 的消息（ADR-0094）。
//!
//! 附件进附件区就开始上传：照片走 Matrix 的媒体上传，文件走文件服务的 tus（ADR-0020）。点发送后这一组等附件全部传完、
//! 文件授权给房间，才按顺序把事件一次交给 matrix-sdk 的发送队列；每条事件带同一个 [`MessageGroup`]。

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::SendState;

/// 一条时间线的输入框：附件区和这条时间线里还没交出去的消息。经 `ClientCore::subscribe_composer` 按时间线订阅；
/// 附件区的操作是 [`TimelineAction`](crate::TimelineAction) 里的 `SendComposer` 等几个。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ComposerViewModel {
    pub target: crate::TimelineTarget,
    /// 附件区，按加入的顺序，也是发出去的顺序。
    pub attachments: Vec<ComposerAttachment>,
    /// 点了发送、还没交给 Matrix 的消息，先点的在前。壳画在时间线最后一条后面；交出去以后由 SDK 的本地回显代替，核心从这里拿掉。
    pub outgoing: Vec<OutgoingMessage>,
    /// 附件区最多放几个，壳据此限制系统选择器能选几个。
    pub max_attachments: u32,
}

/// 附件区里的一项，或者待发消息里的一个附件。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ComposerAttachment {
    /// 本机给这个附件的标识，交给删除、重试的操作和取预览图的方法。
    pub id: String,
    pub kind: AttachmentKind,
    /// 文件名，壳给的。
    pub name: String,
    pub mime_type: Option<String>,
    /// 字节数；文件还没打开时为空。
    pub size: Option<u64>,
    pub state: AttachmentState,
}

/// 照片发成 `m.image`，文件经文件服务发成链接。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AttachmentKind {
    /// 照片：预览图经 `ClientCore::attachment_thumbnail` 取。像素尺寸壳给过才有。
    Image {
        width: Option<u32>,
        height: Option<u32>,
    },
    /// 文件：壳画图标和文件名。
    File,
}

/// 一个附件的上传进展。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum AttachmentState {
    /// 在传，或排着队等传。`progress` 范围 `0.0..=1.0`，不知道总量时为空；重传时可能倒退，壳原样画。
    Uploading { progress: Option<f32> },
    /// 连不上家里的服务器，连上后核心自己接着传。
    Paused,
    /// 传完了。文件还没授权给房间里的 Bot，授权在发送时做。
    Ready,
    /// 传不上去，核心不自动再试：主人重试或者删掉。`reason` 决定壳写的说明，`message` 是默认收起的详情，按 App 的语言。
    Failed {
        reason: AttachmentFailure,
        message: String,
    },
}

/// 附件为什么传不上去。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum AttachmentFailure {
    /// 照片超过 Matrix 服务器的上传上限（`m.upload.size`）。照片发不了，可以改用文件发。
    TooLarge,
    /// 手机上的原文件读不了：被删、被改过，或者系统收回了读权限，要重新选。
    SourceUnavailable,
    /// 家里服务器的磁盘放不下。
    StorageFull,
    /// 家里的服务器拒收：没有权限、请求不合规则。
    Rejected,
    /// 家里的服务器出错。
    Upstream,
    /// 手机这边存不下附件区的内容。
    LocalStorage,
    /// 更新的核心给出的类别：显示详情。
    #[serde(other)]
    Other,
}

/// 点了发送、还没交给 Matrix 的一条消息。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct OutgoingMessage {
    /// 本机给这条消息的标识，交给 `TimelineAction::RetryOutgoing`、`TimelineAction::CancelOutgoing`；也是列表里这一行的 ID，
    /// 不会和时间线条目的 ID 相同。
    pub id: String,
    /// 主人写的字，可以为空。
    pub body: String,
    pub attachments: Vec<ComposerAttachment>,
    /// 点发送的时间，Unix 毫秒。
    pub timestamp_ms: u64,
    /// `Sending` 的 `progress` 是全部附件合起来的比例。有附件传不上去、授权没成时是 `Failed`（`recoverable` 为假），
    /// 原因在 `message` 里，失败的附件也标着 [`AttachmentState::Failed`]。
    pub state: SendState,
}

/// 一次发送里的几条事件带的分组标记（`app.wonton.batch`，ADR-0094 第 3 节）。时间线里由核心解析好给壳；发送端和接收端（ACP 桥）
/// 都用这里的格式和校验。
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct MessageGroup {
    /// 这一组的标识：32 个小写十六进制字符。按（房间、发送者、`id`）认组。
    pub id: String,
    /// 这条在组里的位置，从 0 开始。
    pub index: u32,
    /// 这一组一共几条事件。
    pub count: u32,
}

impl MessageGroup {
    /// 事件内容里放标记的键。
    pub const FIELD: &'static str = "app.wonton.batch";
    /// 结构版本。
    pub const VERSION: u64 = 1;
    /// 一组最多几条事件：发送端最多 10 个附件，接收端多留余地。
    pub const MAX_COUNT: u32 = 32;
    const ID_LEN: usize = 32;

    /// 从事件内容（顶层对象）里读标记。没有、版本不认识或者不合规则时为 `None`，接收方按单独一条处理。
    pub fn from_content(content: &Map<String, Value>) -> Option<Self> {
        let marker = content.get(Self::FIELD)?.as_object()?;
        if marker.get("v")?.as_u64()? != Self::VERSION {
            return None;
        }
        let id = marker.get("id")?.as_str()?;
        let index = u32::try_from(marker.get("index")?.as_u64()?).ok()?;
        let count = u32::try_from(marker.get("count")?.as_u64()?).ok()?;
        let group = Self {
            id: id.to_owned(),
            index,
            count,
        };
        group.is_valid().then_some(group)
    }

    /// 写进事件内容的键和值。
    pub fn to_content(&self) -> (String, Value) {
        let value = serde_json::json!({
            "v": Self::VERSION,
            "id": self.id,
            "index": self.index,
            "count": self.count,
        });
        (Self::FIELD.to_owned(), value)
    }

    /// `id` 合格、`count` 在 1 到 [`Self::MAX_COUNT`]、`index` 小于 `count`。
    pub fn is_valid(&self) -> bool {
        let id_ok = self.id.len() == Self::ID_LEN
            && self
                .id
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
        id_ok && (1..=Self::MAX_COUNT).contains(&self.count) && self.index < self.count
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const ID: &str = "3f0c9a1e5b7d42c6a8e1f0b2c4d6e8a0";

    fn content(value: Value) -> Map<String, Value> {
        let Value::Object(map) = value else {
            panic!("不是对象")
        };
        map
    }

    /// 契约：标记的线上格式（ADR-0094 第 3 节），ACP 桥按它收齐一组。
    #[test]
    fn the_marker_round_trips_through_event_content() {
        let group = MessageGroup {
            id: ID.into(),
            index: 2,
            count: 3,
        };
        let (key, value) = group.to_content();
        assert_eq!(key, "app.wonton.batch");
        assert_eq!(value, json!({"v": 1, "id": ID, "index": 2, "count": 3}));
        let event = content(json!({"msgtype": "m.image", "body": "a.jpg", key: value}));
        assert_eq!(MessageGroup::from_content(&event), Some(group));
    }

    #[test]
    fn markers_that_break_the_rules_count_as_no_marker() {
        let bad = [
            json!({"v": 2, "id": ID, "index": 0, "count": 1}),
            json!({"id": ID, "index": 0, "count": 1}),
            json!({"v": 1, "id": ID, "index": 1, "count": 1}),
            json!({"v": 1, "id": ID, "index": 0, "count": 0}),
            json!({"v": 1, "id": ID, "index": 0, "count": 33}),
            json!({"v": 1, "id": "3F0C9A1E5B7D42C6A8E1F0B2C4D6E8A0", "index": 0, "count": 1}),
            json!({"v": 1, "id": "short", "index": 0, "count": 1}),
            json!({"v": 1, "id": ID, "index": -1, "count": 1}),
            json!("not an object"),
        ];
        for marker in bad {
            let event = content(json!({"body": "x", "app.wonton.batch": marker}));
            assert_eq!(MessageGroup::from_content(&event), None, "{marker}");
        }
        assert_eq!(
            MessageGroup::from_content(&content(json!({"body": "x"}))),
            None
        );
    }

    #[test]
    fn composer_types_round_trip_through_json() {
        let view = ComposerViewModel {
            target: crate::TimelineTarget {
                room_id: "!a:home.example".into(),
                thread_root: None,
            },
            attachments: vec![ComposerAttachment {
                id: "a1".into(),
                kind: AttachmentKind::Image {
                    width: Some(640),
                    height: None,
                },
                name: "cat.jpg".into(),
                mime_type: Some("image/jpeg".into()),
                size: Some(4096),
                state: AttachmentState::Uploading {
                    progress: Some(0.5),
                },
            }],
            outgoing: vec![OutgoingMessage {
                id: "o1".into(),
                body: "哪张好看".into(),
                attachments: vec![ComposerAttachment {
                    id: "a2".into(),
                    kind: AttachmentKind::File,
                    name: "账单.pdf".into(),
                    mime_type: None,
                    size: None,
                    state: AttachmentState::Failed {
                        reason: AttachmentFailure::SourceUnavailable,
                        message: "文件被删了".into(),
                    },
                }],
                timestamp_ms: 1_793_000_000_000,
                state: SendState::Failed {
                    message: "有附件传不上去".into(),
                    recoverable: false,
                },
            }],
            max_attachments: 10,
        };
        let json = serde_json::to_string(&view).unwrap();
        assert_eq!(
            serde_json::from_str::<ComposerViewModel>(&json).unwrap(),
            view
        );
        assert_eq!(
            serde_json::from_value::<AttachmentFailure>(json!("from_the_future")).unwrap(),
            AttachmentFailure::Other
        );
    }
}
