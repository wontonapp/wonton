//! 把 `wonton-matrix` 的结果转成 ViewModel：房间行、时间线条目和增量、加密状态、按原因分类的错误。

use super::confirmation::CardsView;
use super::read_mark::ReadMark;
use super::rich;
use wonton_l10n::l10n;
use wonton_matrix::{
    EntryKind, Error, LatestMessage, Membership, Message, MessageBody, Recovery, RoomInfo,
    SendStatus, TimelineEntry, UserId, VectorDiff, Verification,
};
use wonton_protocol::{
    ChatFailure, CoreError, DeviceVerification, FileLinks, ImageContent, MediaHandle,
    MessageAction, MessageContent, MessageSummary, RecoveryStatus, RoomMembership, RoomPreview,
    RoomRow, SendState, TimelineContent, TimelineDiff, TimelineItem, TimelineMessage,
    extract_file_links,
};

use crate::l10n::{CATALOG, language};

/// `core`：核心账号，和它的私聊是通知房间。
/// 邀请在最前面，其余按最新消息排。还没有消息的放最后，按名字排：SDK 给的顺序在设备之间不一样，同一个账号两台设备要排得一样。
pub(crate) fn sort_rooms(rows: &mut [RoomRow]) {
    rows.sort_by(|a, b| {
        let invited = |row: &RoomRow| row.membership == RoomMembership::Invited;
        let latest = |row: &RoomRow| row.preview.as_ref().map(|preview| preview.timestamp_ms);
        invited(b)
            .cmp(&invited(a))
            .then_with(|| latest(b).cmp(&latest(a)))
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.room_id.cmp(&b.room_id))
    });
}

pub(crate) fn room_row(room: RoomInfo, core: &UserId) -> RoomRow {
    let is_read_only = room.direct_user_id.as_deref() == Some(core);
    RoomRow {
        room_id: room.room_id.to_string(),
        name: room.name,
        is_direct: room.is_direct,
        direct_user_id: room.direct_user_id.map(|user| user.to_string()),
        is_encrypted: room.is_encrypted,
        is_read_only,
        membership: match room.membership {
            Membership::Joined => RoomMembership::Joined,
            Membership::Invited => RoomMembership::Invited,
            Membership::Left | Membership::Knocked | Membership::Banned => RoomMembership::Left,
        },
        preview: room.latest.and_then(preview),
        unread_count: room.unread_count,
    }
}

/// 预览。不是消息的事件（成员变化等）不做预览。
fn preview(latest: LatestMessage) -> Option<RoomPreview> {
    if matches!(&latest.body, MessageBody::Other { kind } if kind == "state") {
        return None;
    }
    let body = summary(latest.body, latest.is_own);
    Some(RoomPreview {
        sender_name: latest.sender_name,
        is_own: latest.is_own,
        body,
        timestamp_ms: latest.timestamp_ms,
    })
}

/// 消息的摘要：房间列表的预览用它。推送的正文是字符串，由 `push::notification` 转换。
pub(crate) fn summary(body: MessageBody, is_own: bool) -> MessageSummary {
    match body {
        MessageBody::Text(text) | MessageBody::Notice(text) => {
            let FileLinks { body, files } = extract_file_links(&text);
            if body.is_empty() && !files.is_empty() {
                MessageSummary::Files
            } else if is_own {
                MessageSummary::Text { text: body }
            } else {
                MessageSummary::Text {
                    text: rich::flatten(&rich::markdown(&body)),
                }
            }
        }
        MessageBody::Image(_) => MessageSummary::Image,
        MessageBody::Undecryptable => MessageSummary::Undecryptable,
        MessageBody::Redacted => MessageSummary::Redacted,
        MessageBody::Other { kind } => MessageSummary::Other { kind },
    }
}

pub(crate) fn timeline_item(
    entry: TimelineEntry,
    cards: &CardsView,
    read: &ReadMark,
) -> TimelineItem {
    let read_by = read.readers_of(&entry.id);
    let content = match entry.kind {
        EntryKind::Message(message) => TimelineContent::Message {
            message: timeline_message(*message, cards, read_by),
        },
        EntryKind::Notice(text) => TimelineContent::Notice { text },
        EntryKind::ProfileChange {
            user_id,
            name,
            previous_name,
            avatar_changed,
        } => TimelineContent::ProfileChange {
            user_id: user_id.to_string(),
            name,
            previous_name,
            avatar_changed,
        },
        EntryKind::DateDivider(timestamp_ms) => TimelineContent::DateDivider { timestamp_ms },
        EntryKind::ReadMarker => TimelineContent::ReadMarker,
        EntryKind::TimelineStart => TimelineContent::TimelineStart,
    };
    TimelineItem {
        id: entry.id,
        content,
    }
}

fn timeline_message(message: Message, cards: &CardsView, read_by: Vec<String>) -> TimelineMessage {
    let card = cards.view_of(&message);
    // 卡片靠编辑更新状态，对主人来说不是“编辑过”。
    let edited = message.edited && card.is_none();
    let body = match (card, message.body) {
        (Some(view), _) => MessageContent::Confirmation { view },
        (None, body) => message_content(body, message.is_own),
    };
    TimelineMessage {
        event_id: message.event_id.map(|id| id.to_string()),
        sender_id: message.sender.to_string(),
        sender_name: message.sender_name,
        is_own: message.is_own,
        timestamp_ms: message.timestamp_ms,
        send_state: match message.status {
            SendStatus::Sending { progress } => SendState::Sending {
                progress: progress.and_then(|p| {
                    (p.total > 0).then(|| (p.current.min(p.total) as f64 / p.total as f64) as f32)
                }),
            },
            SendStatus::Sent => SendState::Sent,
            SendStatus::Failed { error, recoverable } => SendState::Failed {
                message: error,
                recoverable,
            },
        },
        edited,
        actions: message_actions(&body),
        body,
        read_by,
    }
}

fn message_actions(body: &MessageContent) -> Vec<MessageAction> {
    match body {
        MessageContent::Text { body, blocks, .. } if !body.trim().is_empty() => {
            vec![MessageAction::Copy {
                text: rich::copy_text(blocks),
            }]
        }
        _ => Vec::new(),
    }
}

fn message_content(body: MessageBody, is_own: bool) -> MessageContent {
    match body {
        MessageBody::Text(text) | MessageBody::Notice(text) => {
            let FileLinks { body, files } = extract_file_links(&text);
            let blocks = if is_own {
                rich::plain(&body)
            } else {
                rich::markdown(&body)
            };
            MessageContent::Text {
                body,
                blocks,
                files,
            }
        }
        MessageBody::Image(image) => MessageContent::Image {
            image: ImageContent {
                media: MediaHandle {
                    token: image.media_token(),
                },
                thumbnail: image.thumbnail_token().map(|token| MediaHandle { token }),
                filename: image.filename,
                mime_type: image.mime_type,
                size: image.size,
                width: image.width,
                height: image.height,
            },
        },
        MessageBody::Undecryptable => MessageContent::Undecryptable,
        MessageBody::Redacted => MessageContent::Redacted,
        MessageBody::Other { kind } => MessageContent::Unsupported { kind },
    }
}

/// 下标超出 `u32` 的时间线不存在（几十亿条），真遇到就截到最大值。
fn index(index: usize) -> u32 {
    u32::try_from(index).unwrap_or(u32::MAX)
}

pub(crate) fn timeline_diff(
    diff: VectorDiff<TimelineEntry>,
    cards: &CardsView,
    read: &ReadMark,
) -> TimelineDiff {
    let item = |entry| timeline_item(entry, cards, read);
    match diff {
        VectorDiff::Append { values } => TimelineDiff::Append {
            items: values.into_iter().map(item).collect(),
        },
        VectorDiff::Clear => TimelineDiff::Clear,
        VectorDiff::PushFront { value } => TimelineDiff::PushFront { item: item(value) },
        VectorDiff::PushBack { value } => TimelineDiff::PushBack { item: item(value) },
        VectorDiff::PopFront => TimelineDiff::PopFront,
        VectorDiff::PopBack => TimelineDiff::PopBack,
        VectorDiff::Insert { index: at, value } => TimelineDiff::Insert {
            index: index(at),
            item: item(value),
        },
        VectorDiff::Set { index: at, value } => TimelineDiff::Set {
            index: index(at),
            item: item(value),
        },
        VectorDiff::Remove { index: at } => TimelineDiff::Remove { index: index(at) },
        VectorDiff::Truncate { length } => TimelineDiff::Truncate {
            length: index(length),
        },
        VectorDiff::Reset { values } => TimelineDiff::Reset {
            items: values.into_iter().map(item).collect(),
        },
    }
}

pub(crate) fn verification(verification: Verification) -> DeviceVerification {
    match verification {
        Verification::Unknown => DeviceVerification::Unknown,
        Verification::Verified => DeviceVerification::Verified,
        Verification::Unverified => DeviceVerification::Unverified,
    }
}

pub(crate) fn recovery(recovery: Recovery) -> RecoveryStatus {
    match recovery {
        Recovery::Unknown => RecoveryStatus::Unknown,
        Recovery::Disabled => RecoveryStatus::Disabled,
        Recovery::Enabled => RecoveryStatus::Enabled,
        Recovery::Incomplete => RecoveryStatus::Incomplete,
    }
}

/// 连不上时主人看到的话。连接的情况由 `HomeViewModel` 说，原文只进日志（ADR-0032）。
pub(crate) fn not_connected() -> String {
    l10n!(CATALOG, language(), "chat-not-connected")
}

/// Matrix 错误的原因。
pub(crate) fn failure(error: &Error) -> ChatFailure {
    match error {
        Error::Transport(_) => ChatFailure::TransportUnavailable,
        Error::Unauthorized { .. } => ChatFailure::Unauthorized,
        Error::NotReady(_) => ChatFailure::NotReady,
        Error::Upstream(_) => ChatFailure::Upstream,
        Error::InvalidInput(_) => ChatFailure::InvalidInput,
    }
}

/// 直接返回给壳的 Matrix 错误。
pub(crate) fn core_error(error: &Error) -> CoreError {
    let reason = error.to_string();
    match failure(error) {
        ChatFailure::TransportUnavailable => CoreError::TransportUnavailable {
            reason: not_connected(),
        },
        ChatFailure::Unauthorized => CoreError::Unauthorized { reason },
        ChatFailure::NotReady => CoreError::NotReady { reason },
        ChatFailure::Upstream => CoreError::Upstream { reason },
        ChatFailure::InvalidInput => CoreError::InvalidInput { reason },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wonton_matrix::UploadProgress;
    use wonton_protocol::FileId;

    #[test]
    fn timeline_exposes_upload_fractions_without_hiding_regressions() {
        let owner: wonton_matrix::OwnedUserId = "@owner:wonton.localhost".parse().unwrap();
        let (_, answers) = tokio::sync::watch::channel(Default::default());
        let cards = crate::chat::confirmation::Cards::new(&owner, answers).view();
        for (counters, expected) in [
            (None, None),
            (Some((0, 0)), None),
            (Some((8, 0)), None),
            (Some((0, 8)), Some(0.0)),
            (Some((4, 8)), Some(0.5)),
            (Some((2, 8)), Some(0.25)),
            (Some((8, 8)), Some(1.0)),
            (Some((9, 8)), Some(1.0)),
            (Some((u64::MAX, 1)), Some(1.0)),
            (Some((u64::MAX, u64::MAX)), Some(1.0)),
        ] {
            let item = timeline_item(
                entry(SendStatus::Sending {
                    progress: counters.map(|(current, total)| UploadProgress { current, total }),
                }),
                &cards,
                &ReadMark::default(),
            );
            let TimelineContent::Message { message } = item.content else {
                panic!("the message must stay in the timeline");
            };
            assert_eq!(
                message.send_state,
                SendState::Sending { progress: expected }
            );
            if let SendState::Sending {
                progress: Some(value),
            } = message.send_state
            {
                assert!(value.is_finite() && (0.0..=1.0).contains(&value));
            }
            assert!(message.event_id.is_none());
        }
    }

    fn entry(status: SendStatus) -> TimelineEntry {
        TimelineEntry {
            id: "upload".to_owned(),
            kind: EntryKind::Message(Box::new(Message {
                event_id: None,
                sender: "@owner:wonton.localhost".parse().unwrap(),
                sender_name: "主人".to_owned(),
                is_own: true,
                timestamp_ms: 0,
                status,
                body: MessageBody::Text("attachment".to_owned()),
                custom: Default::default(),
                mentions: Vec::new(),
                thread_root: None,
                edited: false,
                read_by: Vec::new(),
            })),
        }
    }

    #[test]
    fn previews_do_not_show_file_links() {
        let link = FileId::from_bytes([1; 16]).link();
        let with_text = summary(MessageBody::Text(format!("给你 {link}")), false);
        assert_eq!(
            with_text,
            MessageSummary::Text {
                text: "给你".to_owned()
            }
        );
        let only_link = summary(MessageBody::Text(link), false);
        assert_eq!(only_link, MessageSummary::Files);
    }
}
