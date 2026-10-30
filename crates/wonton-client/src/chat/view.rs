//! 把 `wonton-matrix` 的结果转成 ViewModel：房间行、时间线条目和增量、加密状态、按原因分类的错误。

use wonton_matrix::{
    EntryKind, Error, LatestMessage, Membership, Message, MessageBody, Recovery, RoomInfo,
    SendStatus, TimelineEntry, VectorDiff, Verification,
};
use wonton_protocol::{
    ChatFailure, CoreError, DeviceVerification, ImageContent, MediaHandle, MessageContent,
    RecoveryStatus, RoomMembership, RoomPreview, RoomRow, SendState, TimelineContent, TimelineDiff,
    TimelineItem, TimelineMessage,
};

pub(crate) fn room_row(room: RoomInfo) -> RoomRow {
    RoomRow {
        room_id: room.room_id.to_string(),
        name: room.name,
        is_direct: room.is_direct,
        direct_user_id: room.direct_user_id.map(|user| user.to_string()),
        is_encrypted: room.is_encrypted,
        membership: match room.membership {
            Membership::Joined => RoomMembership::Joined,
            Membership::Invited => RoomMembership::Invited,
            Membership::Left | Membership::Knocked | Membership::Banned => RoomMembership::Left,
        },
        preview: room.latest.and_then(preview),
        unread_count: room.unread_count,
    }
}

/// 预览的一行文字。不是消息的事件（成员变化等）不做预览。
fn preview(latest: LatestMessage) -> Option<RoomPreview> {
    if matches!(&latest.body, MessageBody::Other { kind } if kind == "state") {
        return None;
    }
    let text = summary(latest.body);
    Some(RoomPreview {
        sender_name: latest.sender_name,
        is_own: latest.is_own,
        text,
        timestamp_ms: latest.timestamp_ms,
    })
}

/// 消息的一行文字：房间列表的预览和通知的正文都用它。
pub(crate) fn summary(body: MessageBody) -> String {
    match body {
        MessageBody::Text(text) => text,
        MessageBody::Image(_) => "[图片]".to_owned(),
        MessageBody::Undecryptable => "[加密消息]".to_owned(),
        MessageBody::Redacted => "[已撤回]".to_owned(),
        MessageBody::Other { kind } => format!("[{kind}]"),
    }
}

pub(crate) fn timeline_item(entry: TimelineEntry) -> TimelineItem {
    let content = match entry.kind {
        EntryKind::Message(message) => TimelineContent::Message {
            message: timeline_message(*message),
        },
        EntryKind::Notice(text) => TimelineContent::Notice { text },
        EntryKind::DateDivider(timestamp_ms) => TimelineContent::DateDivider { timestamp_ms },
        EntryKind::ReadMarker => TimelineContent::ReadMarker,
        EntryKind::TimelineStart => TimelineContent::TimelineStart,
    };
    TimelineItem {
        id: entry.id,
        content,
    }
}

fn timeline_message(message: Message) -> TimelineMessage {
    TimelineMessage {
        event_id: message.event_id.map(|id| id.to_string()),
        sender_id: message.sender.to_string(),
        sender_name: message.sender_name,
        is_own: message.is_own,
        timestamp_ms: message.timestamp_ms,
        send_state: match message.status {
            SendStatus::Sending => SendState::Sending,
            SendStatus::Sent => SendState::Sent,
            SendStatus::Failed { error, recoverable } => SendState::Failed {
                message: error,
                recoverable,
            },
        },
        body: match message.body {
            MessageBody::Text(body) => MessageContent::Text { body },
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
        },
        edited: message.edited,
        read_by: message.read_by,
    }
}

/// 下标超出 `u32` 的时间线不存在（几十亿条），真遇到就截到最大值。
fn index(index: usize) -> u32 {
    u32::try_from(index).unwrap_or(u32::MAX)
}

pub(crate) fn timeline_diff(diff: VectorDiff<TimelineEntry>) -> TimelineDiff {
    match diff {
        VectorDiff::Append { values } => TimelineDiff::Append {
            items: values.into_iter().map(timeline_item).collect(),
        },
        VectorDiff::Clear => TimelineDiff::Clear,
        VectorDiff::PushFront { value } => TimelineDiff::PushFront {
            item: timeline_item(value),
        },
        VectorDiff::PushBack { value } => TimelineDiff::PushBack {
            item: timeline_item(value),
        },
        VectorDiff::PopFront => TimelineDiff::PopFront,
        VectorDiff::PopBack => TimelineDiff::PopBack,
        VectorDiff::Insert { index: at, value } => TimelineDiff::Insert {
            index: index(at),
            item: timeline_item(value),
        },
        VectorDiff::Set { index: at, value } => TimelineDiff::Set {
            index: index(at),
            item: timeline_item(value),
        },
        VectorDiff::Remove { index: at } => TimelineDiff::Remove { index: index(at) },
        VectorDiff::Truncate { length } => TimelineDiff::Truncate {
            length: index(length),
        },
        VectorDiff::Reset { values } => TimelineDiff::Reset {
            items: values.into_iter().map(timeline_item).collect(),
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
        ChatFailure::TransportUnavailable => CoreError::TransportUnavailable { reason },
        ChatFailure::Unauthorized => CoreError::Unauthorized { reason },
        ChatFailure::NotReady => CoreError::NotReady { reason },
        ChatFailure::Upstream => CoreError::Upstream { reason },
        ChatFailure::InvalidInput => CoreError::InvalidInput { reason },
    }
}
