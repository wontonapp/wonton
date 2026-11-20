//! 按时间线（主时间线或旁聊）发给核心的操作。ADR-0093 §5。

use serde::{Deserialize, Serialize};

use crate::{ImageUpload, TimelineScroll};

/// 操作针对的时间线：主时间线或一个旁聊（Matrix 话题）。
///
/// 同一房间的主时间线和每个旁聊的条目、草稿、位置、已读都按 `thread_root` 分开记。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct TimelineTarget {
    /// 房间 ID。
    pub room_id: String,
    /// 旁聊根消息的事件 ID；空是主时间线。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_root: Option<String>,
}

/// 发给某条时间线的操作。
///
/// 原来直接放在 [`crate::Action`] 上；`Action` 的 UniFFI 元数据一个类型只有 32 KiB，成组的
/// 操作整组搬进嵌套枚举，嵌套枚举有自己的 32 KiB（ADR-0093 §5）。以后要加一组相关操作，同样
/// 先搬成一个嵌套枚举，不在 `Action` 上逐个加变体。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TimelineAction {
    /// 发送文字。正文去掉首尾空白后不能为空。消息先以本地回显出现在时间线里。
    ///
    /// `reply_to` 是被回复消息的事件 ID（[`MessageAction::Reply`](crate::MessageAction::Reply) 里的），发出的是对它的
    /// 回复；那条消息不在这个房间里时返回 `InvalidInput`。
    ///
    /// `mentions` 是主人在输入框的“@”列表里选中的人（完整的 Matrix 用户 ID）：核心写进 `m.mentions.user_ids`，
    /// `formatted_body` 里按正文里的 `@显示名` 放胶囊，被回复的人在回复时也会加进来。只认选中的人，不从正文按
    /// 显示名猜（ADR-0083）。不是房间成员的、重复的忽略。
    SendText {
        target: TimelineTarget,
        body: String,
        #[cfg_attr(feature = "uniffi", uniffi(default = None))]
        reply_to: Option<String>,
        #[serde(default)]
        #[cfg_attr(feature = "uniffi", uniffi(default))]
        mentions: Vec<String>,
    },
    /// 发送图片。上传和发送的进展、`reply_to` 同 [`TimelineAction::SendText`]。
    ///
    /// `caption` 是图片下面带的一段话。旁聊里贴图（ADR-0109 第 8 节）用它把输入框里已经打的字一起发出去，
    /// 发完清掉草稿；主时间线有附件区，贴图走 [`TimelineAction::SendComposer`]。
    SendImage {
        target: TimelineTarget,
        image: ImageUpload,
        #[cfg_attr(feature = "uniffi", uniffi(default = None))]
        caption: Option<String>,
        #[cfg_attr(feature = "uniffi", uniffi(default = None))]
        reply_to: Option<String>,
    },
    /// 把一条消息转发到另一个房间（ADR-0073）：照原样再发一条，不带“转发自”，加密附件不重新上传。`item_id` 同
    /// [`TimelineAction::RetrySend`]，只能转发菜单里有 [`MessageAction::Forward`](crate::MessageAction::Forward) 的消息。
    /// 进展在目标房间的时间线里（本地回显、发送状态），壳随后打开目标房间。目标是 `to_room_id` 的主时间线，
    /// 不往旁聊里转发。
    ForwardMessage {
        target: TimelineTarget,
        item_id: String,
        to_room_id: String,
    },
    /// 撤回一条已经发出去的消息（Matrix 的 redaction），所有人那里都变成 [`MessageContent::Redacted`](crate::MessageContent::Redacted)。
    /// 只能撤回菜单里有 [`MessageAction::Redact`](crate::MessageAction::Redact) 的消息；还没发出去的用
    /// [`TimelineAction::CancelSend`]。失败进 `ChatViewModel::problem`。
    RedactMessage {
        target: TimelineTarget,
        item_id: String,
    },
    /// 马上重发一条发送失败的消息（[`SendState::Failed`](crate::SendState::Failed)），不等核心自动重发。`item_id` 是
    /// 它的 [`TimelineItem::id`](crate::TimelineItem::id)，要先订阅这条时间线。
    RetrySend {
        target: TimelineTarget,
        item_id: String,
    },
    /// 取消发送：删掉一条还没发出去的消息（排队中或发送失败的），同一时间线排在它后面的接着发。`item_id` 同
    /// [`TimelineAction::RetrySend`]。已经发出去的取消不了。
    CancelSend {
        target: TimelineTarget,
        item_id: String,
    },
    /// 进入编辑态（ADR-0109 第 8 节）：只能编辑菜单里有 [`MessageAction::Edit`](crate::MessageAction::Edit) 的消息。
    /// 核心把现在的草稿（正文和回复对象）存下来，换成正被编辑的那条的正文，记进
    /// [`TimelineUpdate::editing`](crate::TimelineUpdate::editing)。已经在编辑别的消息时直接换过去，不叠加。
    StartEdit {
        target: TimelineTarget,
        item_id: String,
    },
    /// 退出编辑态、不发这条修改：恢复进入编辑态之前的草稿（正文和回复对象）。没在编辑时什么也不做。
    CancelEdit { target: TimelineTarget },
    /// 确认这次修改（[`TimelineAction::StartEdit`] 之后），矩阵上发到 `event_id`（现在这条），正文换成 `body`（空正文
    /// 返回 `InvalidInput`）。发出去的事件在所有人那里整条替换成新正文（ADR-0109 第 8 节）；成功后清掉编辑态、草稿
    /// 换回进入编辑态之前的，失败时编辑态留着、失败原因进 `ChatViewModel::problem`。新正文按
    /// ADR-0054 的 Markdown 记成格式化正文。
    EditMessage {
        target: TimelineTarget,
        event_id: String,
        body: String,
    },
    /// 壳报告输入框在不在打字（ADR-0109 第 7 节）：正文有变化发 `true`，输入框失焦、清空、主人离开这条时间线或
    /// 发送后发 `false`。核心按需转发成“正在输入”到服务器，同一条时间线里至少隔几秒才再发一次，停止输入几秒后
    /// 自动转成“停了”，壳不用自己计时。
    SetTyping {
        target: TimelineTarget,
        typing: bool,
    },
    /// 加载更早的消息。要先订阅这条时间线，结果体现在时间线的增量和翻页状态里。
    PaginateBackwards { target: TimelineTarget },
    /// 壳报告列表停在哪儿：滚动停下、进出底部时发，不用每一帧发。要先订阅这条时间线。核心据此重算
    /// [`TimelineUpdate::viewport`](crate::TimelineUpdate::viewport)，下面的新消息数随之变。
    SetTimelineScroll {
        target: TimelineTarget,
        scroll: TimelineScroll,
    },
    /// 把这条时间线标为读到最新一条，发出已读回执（旁聊是话题已读回执，主时间线是普通回执），未读数随之清零。
    /// 只在列表真的贴着最新一条时发（新消息到了、滚回底部），翻在别处时不发。
    MarkRead { target: TimelineTarget },
    /// 记下这条时间线输入框里的草稿，主人换个房间、关掉 App 再回来还在；正文为空、也没在回复时表示清掉。
    /// 只存本机，不上服务器。发送成功后核心自己清掉。旁聊的草稿和主时间线的分开。`reply_to` 是正在回复的消息的
    /// 事件 ID，读回时在 [`ComposerDraft::reply`](crate::ComposerDraft::reply)。
    SetDraft {
        target: TimelineTarget,
        body: String,
        #[cfg_attr(feature = "uniffi", uniffi(default = None))]
        reply_to: Option<String>,
    },
    /// 把输入框的字和附件区的附件作为一次发送发出（ADR-0094）：马上变成一条待发消息
    /// （[`ComposerViewModel::outgoing`](crate::ComposerViewModel::outgoing)），附件区和草稿清空；附件全部传完、文件授权给
    /// 房间后，才按顺序把这一组事件交给 Matrix。`reply_to`、`mentions` 同 [`TimelineAction::SendText`]。字和附件都没有、
    /// 附件区里有失败的附件时返回 `InvalidInput`。
    SendComposer {
        target: TimelineTarget,
        body: String,
        #[cfg_attr(feature = "uniffi", uniffi(default = None))]
        reply_to: Option<String>,
        #[serde(default)]
        #[cfg_attr(feature = "uniffi", uniffi(default))]
        mentions: Vec<String>,
    },
    /// 从附件区删掉一个附件，停下它的上传。已经传上去的照片留在服务器上，文件由家里的服务器到期清掉。附件区里没有它时
    /// 返回 `InvalidInput`。
    RemoveAttachment {
        target: TimelineTarget,
        attachment_id: String,
    },
    /// 重新上传一个失败的附件；不是失败的不动。附件区和待发消息里都没有它时返回 `InvalidInput`。
    RetryAttachment {
        target: TimelineTarget,
        attachment_id: String,
    },
    /// 重发一条失败的待发消息：只重传失败的附件，再接着发。不是失败的不动；找不到时返回 `InvalidInput`。
    RetryOutgoing {
        target: TimelineTarget,
        outgoing_id: String,
    },
    /// 取消一条还没交出去的待发消息，整条不发，停下它的上传；排在它后面的接着发。找不到时（已经交出去了）返回
    /// `InvalidInput`，交出去的用 [`TimelineAction::CancelSend`]。
    CancelOutgoing {
        target: TimelineTarget,
        outgoing_id: String,
    },
    /// 点了气泡里的引用：跳到原消息（ADR-0109）。`event_id` 是引用里原消息的事件 ID，`from_item_id` 是被点的那条
    /// （[`TimelineItem::id`](crate::TimelineItem::id)），核心把它压进返回栈（[`TimelineUpdate::back_to`](crate::TimelineUpdate::back_to)）。
    /// 原消息已加载时下一批就带 [`TimelineUpdate::focus`](crate::TimelineUpdate::focus)；没加载时核心往前翻页，进展在
    /// [`TimelineUpdate::jump`](crate::TimelineUpdate::jump)。要先订阅这条时间线。
    JumpToMessage {
        target: TimelineTarget,
        event_id: String,
        from_item_id: String,
    },
    /// “向下”按钮在有 [`TimelineUpdate::back_to`](crate::TimelineUpdate::back_to) 时发：核心弹出栈顶，下一批的
    /// [`TimelineUpdate::focus`](crate::TimelineUpdate::focus) 是它。那一条已经不在列表里了（撤回、被裁掉）时继续弹，
    /// 栈空了就回到底部（[`TimelineUpdate::scroll_to_bottom`](crate::TimelineUpdate::scroll_to_bottom)）。
    JumpBack { target: TimelineTarget },
    /// 去掉发送框当前的链接预览（ADR-0065）。行为细节在 `ClientCore::draft` 的返回和
    /// [`LinkPreviewState`](crate::LinkPreviewState) 上：这条草稿不再出预览，第一个链接换成别的或草稿发出去就作废。
    /// 旁聊的输入框也有链接预览，预览状态和主时间线的分开。
    DismissLinkPreview { target: TimelineTarget },
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn target() -> TimelineTarget {
        TimelineTarget {
            room_id: "!a:home.example".into(),
            thread_root: None,
        }
    }

    /// 契约：旧壳不带 `mentions` 时按没选人处理；带上时原样到核心。
    #[test]
    fn send_actions_mentions_are_optional_and_round_trip() {
        let old: TimelineAction = serde_json::from_value(json!({
            "type": "send_text",
            "target": target(),
            "body": "在吗",
        }))
        .unwrap();
        assert!(matches!(old, TimelineAction::SendText { mentions, .. } if mentions.is_empty()));

        let action = TimelineAction::SendComposer {
            target: target(),
            body: "@Hermes你好".into(),
            reply_to: None,
            mentions: vec!["@hermes:home.example".into()],
        };
        let json = serde_json::to_value(&action).unwrap();
        assert_eq!(json["mentions"], json!(["@hermes:home.example"]));
        assert_eq!(
            serde_json::from_value::<TimelineAction>(json).unwrap(),
            action
        );

        let again: TimelineAction = serde_json::from_value(json!({
            "type": "send_composer",
            "target": target(),
            "body": "x",
        }))
        .unwrap();
        assert!(
            matches!(again, TimelineAction::SendComposer { mentions, .. } if mentions.is_empty())
        );
    }

    /// 契约：编辑、正在输入的操作能经 JSON 往返，壳照线上的字段名发就认。
    #[test]
    fn edit_and_typing_actions_round_trip_through_json() {
        for (action, json) in [
            (
                TimelineAction::StartEdit {
                    target: target(),
                    item_id: "item-1".into(),
                },
                serde_json::json!({
                    "type": "start_edit",
                    "target": {"room_id": "!a:home.example"},
                    "item_id": "item-1",
                }),
            ),
            (
                TimelineAction::CancelEdit { target: target() },
                serde_json::json!({"type": "cancel_edit", "target": {"room_id": "!a:home.example"}}),
            ),
            (
                TimelineAction::EditMessage {
                    target: target(),
                    event_id: "$event".into(),
                    body: "改过的话".into(),
                },
                serde_json::json!({
                    "type": "edit_message",
                    "target": {"room_id": "!a:home.example"},
                    "event_id": "$event",
                    "body": "改过的话",
                }),
            ),
            (
                TimelineAction::SetTyping {
                    target: target(),
                    typing: true,
                },
                serde_json::json!({
                    "type": "set_typing",
                    "target": {"room_id": "!a:home.example"},
                    "typing": true,
                }),
            ),
        ] {
            assert_eq!(serde_json::to_value(&action).unwrap(), json);
            assert_eq!(
                serde_json::from_value::<TimelineAction>(json).unwrap(),
                action
            );
        }
    }

    /// 契约：图片的说明文字能经 JSON 往返，老壳不带 `caption` 时是 None。
    #[test]
    fn image_caption_round_trips_and_may_be_absent() {
        let action = TimelineAction::SendImage {
            target: target(),
            image: ImageUpload {
                filename: "a.jpg".into(),
                mime_type: "image/jpeg".into(),
                data: vec![1, 2],
                width: None,
                height: None,
                thumbnail: None,
            },
            caption: Some("说明".into()),
            reply_to: None,
        };
        let json = serde_json::to_value(&action).unwrap();
        assert_eq!(json["caption"], "说明");
        assert_eq!(
            serde_json::from_value::<TimelineAction>(json.clone()).unwrap(),
            action
        );
        let mut without = json;
        without.as_object_mut().unwrap().remove("caption");
        let parsed = serde_json::from_value::<TimelineAction>(without).unwrap();
        assert!(matches!(
            parsed,
            TimelineAction::SendImage { caption: None, .. }
        ));
    }
}
