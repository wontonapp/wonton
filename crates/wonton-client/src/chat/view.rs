//! 把 `wonton-matrix` 的结果转成 ViewModel：房间行、时间线条目和增量、加密状态、按原因分类的错误。

use std::collections::HashSet;

use super::agent_run::BotsView;
use super::links::LinkTargets;
use super::menu::message_actions;
use super::read_mark::ReadMark;
use super::reply::{quoted_content, reply_quote};
use super::rich;
use wonton_l10n::l10n;
use wonton_matrix::{
    EntryKind, Error, LatestMessage, Membership, Message, MessageBody, Recovery, RoomInfo,
    SendStatus, TimelineEntry, UserId, VectorDiff, Verification,
};
use wonton_protocol::{
    ChatFailure, ChatLayout, CoreError, DeviceVerification, EncryptionStatus, FileLinks,
    ImageCaption, ImageContent, KeyNotice, MediaHandle, MessageContent, MessageSummary,
    QuotedMessage, RecoveryStatus, RichBlock, RoomMembership, RoomPreview, RoomRow, SendState,
    TimelineContent, TimelineDiff, TimelineItem, TimelineMessage, extract_file_links,
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

/// 所有房间未读消息的总数，邀请按 1 计（ADR-0101 §3）。启动器角标只写这个数，不自己按通知条数加。
pub(crate) fn unread_total(view: &wonton_protocol::ChatViewModel) -> u32 {
    view.rooms.iter().fold(0_u32, |total, room| {
        let count = match room.membership {
            RoomMembership::Invited => 1,
            _ => u32::try_from(room.unread_count).unwrap_or(u32::MAX),
        };
        total.saturating_add(count)
    })
}

/// 时间线里有解不开的消息时给主人的下一步。本设备已经能拿到密钥（恢复完成、正在恢复）或者账号里没有别的路时为空。
///
/// 账号设过备份就输恢复密钥：备份带得回旧消息，别的设备确认不一定有。账号没有备份时，只有另一台已验证的设备能把
/// 密钥直接发过来。
pub(crate) fn key_notice(
    encryption: &EncryptionStatus,
    has_undecryptable: bool,
) -> Option<KeyNotice> {
    // 交接期间新设备在等另一台设备交出密钥，不提恢复密钥（ADR-0087）。
    if !has_undecryptable || super::handoff::receiving(encryption).is_some() {
        return None;
    }
    match encryption.recovery {
        RecoveryStatus::Incomplete => Some(KeyNotice::EnterRecoveryKey),
        RecoveryStatus::Disabled if encryption.can_verify_with_other_device => {
            Some(KeyNotice::VerifyWithOtherDevice)
        }
        _ => None,
    }
}

/// `side_chats`：房间里有认得的 Bot 声明了支持旁聊（ADR-0093 §1）。
pub(crate) fn room_row(
    room: RoomInfo,
    core: &UserId,
    layout: ChatLayout,
    bots: &HashSet<String>,
    side_chats: bool,
) -> RoomRow {
    let is_read_only = room.direct_user_id.as_deref() == Some(core);
    RoomRow {
        layout,
        room_id: room.room_id.to_string(),
        name: room.name,
        is_direct: room.is_direct,
        direct_user_id: room.direct_user_id.map(|user| user.to_string()),
        plugin: None,
        is_encrypted: room.is_encrypted,
        is_read_only,
        membership: match room.membership {
            Membership::Joined => RoomMembership::Joined,
            Membership::Invited => RoomMembership::Invited,
            Membership::Left | Membership::Knocked | Membership::Banned => RoomMembership::Left,
        },
        preview: room
            .latest
            .and_then(|latest| preview(latest, bots, side_chats)),
        side_chats,
        unread_count: room.unread_count,
    }
}

/// 预览。不是消息的事件（成员变化等）不做预览。旁聊的根、结果行和话题里的消息给旁聊的标题和状态
/// （ADR-0093 §4）；只看这一条定不了“没收到结果”，那要看话题里的情况，预览里按结果行或开着算。
fn preview(latest: LatestMessage, bots: &HashSet<String>, side_chats: bool) -> Option<RoomPreview> {
    if matches!(&latest.body, MessageBody::Other { kind } if kind == "state") {
        return None;
    }
    if let Some(side_chat) = &latest.side_chat {
        let state = super::side_chat::card_state(side_chats, side_chat.result.as_ref(), None, 0);
        return Some(RoomPreview {
            sender_name: latest.sender_name,
            is_own: latest.is_own,
            body: MessageSummary::SideChat {
                title: side_chat.title.clone(),
                state,
            },
            timestamp_ms: latest.timestamp_ms,
        });
    }
    // Agent 的状态行不进预览（ADR-0082）：状态对象在 SDK 的最新消息里读不到，只能按“Bot 发的提示消息”认，
    // 显示不出上一条就先空着，答案到了预览跟着换。
    if matches!(&latest.body, MessageBody::Notice(_)) && bots.contains(latest.sender_id.as_str()) {
        return None;
    }
    let body = summary(latest.body, latest.is_own, &LinkTargets::default());
    Some(RoomPreview {
        sender_name: latest.sender_name,
        is_own: latest.is_own,
        body,
        timestamp_ms: latest.timestamp_ms,
    })
}

/// 消息的摘要：房间列表的预览用它。推送的正文是字符串，由 `push::notification` 转换。
pub(crate) fn summary(body: MessageBody, is_own: bool, links: &LinkTargets) -> MessageSummary {
    match body {
        MessageBody::Text(text) | MessageBody::Notice(text) => {
            let FileLinks { body, files } = extract_file_links(&text);
            if body.is_empty() && !files.is_empty() {
                MessageSummary::Files
            } else if is_own {
                MessageSummary::Text { text: body }
            } else {
                MessageSummary::Text {
                    text: rich::flatten(&rich::markdown(&body, links)),
                }
            }
        }
        MessageBody::Image(_) => MessageSummary::Image,
        // App 不渲染文件消息；按不认识的消息类型一样显示，交给 Agent 是桥的事。
        MessageBody::File(_) => MessageSummary::Other {
            kind: "m.file".to_owned(),
        },
        MessageBody::Undecryptable => MessageSummary::Undecryptable,
        MessageBody::Redacted => MessageSummary::Redacted,
        MessageBody::Other { kind } => MessageSummary::Other { kind },
    }
}

pub(crate) fn timeline_item(
    entry: TimelineEntry,
    read: &ReadMark,
    links: &LinkTargets,
    bots: &BotsView,
    previews: &super::previews::Previews,
) -> TimelineItem {
    let mark = read.of_entry(&entry.id);
    let content = match entry.kind {
        EntryKind::Message(message) => match &message.agent_run {
            // 只有核心认得的 Bot 发的状态对象才是 AgentRun，别人发的按普通消息显示（ADR-0082 第 4 节）。
            Some(status) if bots.is_bot(message.sender.as_str()) => {
                let body = match &message.body {
                    MessageBody::Text(text) | MessageBody::Notice(text) => text.clone(),
                    _ => String::new(),
                };
                bots.content(status, &body, message.sender.as_str(), &message.sender_name)
            }
            // 旁聊的结果行和带标记的根（ADR-0093 §4）。
            _ => match bots.side_chats().content(&message) {
                Some(content) => content,
                None => TimelineContent::Message {
                    message: timeline_message(*message, mark, links, previews, bots),
                },
            },
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

fn timeline_message(
    message: Message,
    mark: super::read_mark::EntryMark,
    links: &LinkTargets,
    previews: &super::previews::Previews,
    bots: &BotsView,
) -> TimelineMessage {
    let side_chat = bots.side_chats().card_of(&message);
    // 卡片不再是时间线里的消息（ADR-0016）：旧的核心发的卡片和别的消息一样按正文显示，这里没有特殊处理。
    // 预览只给文字消息：事件里带的直接用，没有的用核心取回的缓存（还在取的下一批带上）。
    let preview = super::previews::message_preview(&message, links, previews);
    let body = message_content(message.body, message.is_own, links);
    let event_id = message.event_id.map(|id| id.to_string());
    let sender_id = message.sender.to_string();
    let actions = message_actions(&body, event_id.as_deref(), message.can_redact, |event_id| {
        QuotedMessage {
            event_id,
            sender_id: sender_id.clone(),
            sender_name: message.sender_name.clone(),
            content: quoted_content(&body),
        }
    });
    TimelineMessage {
        event_id,
        sender_id,
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
        edited: message.edited,
        reply: message.reply.map(|reply| reply_quote(reply, links)),
        side_chat,
        preview,
        actions,
        body,
        read_by: mark.readers,
        group: message.group,
        receipt: mark.receipt,
        starts_time_group: mark.starts_time_group,
    }
}

pub(super) fn message_content(
    body: MessageBody,
    is_own: bool,
    links: &LinkTargets,
) -> MessageContent {
    match body {
        MessageBody::Text(text) | MessageBody::Notice(text) => {
            let FileLinks { body, files } = extract_file_links(&text);
            let blocks = blocks_of(&body, is_own, links);
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
                caption: image.caption.map(|body| ImageCaption {
                    blocks: blocks_of(&body, is_own, links),
                    body,
                }),
            },
        },
        MessageBody::Undecryptable => MessageContent::Undecryptable,
        MessageBody::Redacted => MessageContent::Redacted,
        MessageBody::File(_) => MessageContent::Unsupported {
            kind: "m.file".to_owned(),
        },
        MessageBody::Other { kind } => MessageContent::Unsupported { kind },
    }
}

fn blocks_of(body: &str, is_own: bool, links: &LinkTargets) -> Vec<RichBlock> {
    if is_own {
        rich::plain(body, links)
    } else {
        rich::markdown(body, links)
    }
}

/// 下标超出 `u32` 的时间线不存在（几十亿条），真遇到就截到最大值。
fn index(index: usize) -> u32 {
    u32::try_from(index).unwrap_or(u32::MAX)
}

pub(crate) fn timeline_diff(
    diff: VectorDiff<TimelineEntry>,
    read: &ReadMark,
    links: &LinkTargets,
    bots: &BotsView,
    previews: &super::previews::Previews,
) -> TimelineDiff {
    let item = |entry| timeline_item(entry, read, links, bots, previews);
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
        // 不让做这件事（没有权限、对方屏蔽、联邦不通），登录还有效，不能说成登录失效。
        Error::Forbidden { .. } => ChatFailure::Forbidden,
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
        ChatFailure::Forbidden => CoreError::Forbidden { reason },
        ChatFailure::NotReady => CoreError::NotReady { reason },
        ChatFailure::Upstream | ChatFailure::Other => CoreError::Upstream { reason },
        ChatFailure::InvalidInput => CoreError::InvalidInput { reason },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wonton_matrix::UploadProgress;
    use wonton_protocol::{FileId, RichLinkKind};

    fn encryption(
        recovery: RecoveryStatus,
        can_verify_with_other_device: bool,
    ) -> EncryptionStatus {
        EncryptionStatus {
            verification: DeviceVerification::Unverified,
            recovery,
            recovery_key: None,
            backup_reminder: None,
            can_verify_with_other_device,
            can_reset: false,
            recovery_next_step: wonton_protocol::RecoveryNextStep::None,
            verification_flow: None,
            key_handoff: None,
        }
    }

    /// 行为：解不开的消息只在账号里有现成的路时给入口；设过备份走恢复密钥，没设过才让别的设备确认。
    #[test]
    fn the_key_notice_offers_the_way_the_account_actually_has() {
        for (recovery, can_verify, expected) in [
            (
                RecoveryStatus::Incomplete,
                true,
                Some(KeyNotice::EnterRecoveryKey),
            ),
            (
                RecoveryStatus::Incomplete,
                false,
                Some(KeyNotice::EnterRecoveryKey),
            ),
            (
                RecoveryStatus::Disabled,
                true,
                Some(KeyNotice::VerifyWithOtherDevice),
            ),
            (RecoveryStatus::Disabled, false, None),
            (RecoveryStatus::Enabled, true, None),
            (RecoveryStatus::InProgress, true, None),
            (RecoveryStatus::Unknown, true, None),
        ] {
            assert_eq!(
                key_notice(&encryption(recovery, can_verify), true),
                expected,
                "{recovery:?} {can_verify}"
            );
        }
        assert_eq!(
            key_notice(&encryption(RecoveryStatus::Incomplete, true), false),
            None
        );
    }

    #[test]
    fn timeline_exposes_upload_fractions_without_hiding_regressions() {
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
                &ReadMark::default(),
                &LinkTargets::default(),
                &BotsView::default(),
                &super::super::previews::Previews::new(),
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
                agent_run: None,
                thread_root: None,
                thread_replies: None,
                edited: false,
                read_by: Vec::new(),
                reply: None,
                group: None,
                can_redact: true,
            })),
        }
    }

    #[test]
    fn previews_do_not_show_file_links() {
        let link = FileId::from_bytes([1; 16]).link();
        let links = LinkTargets::default();
        let with_text = summary(MessageBody::Text(format!("给你 {link}")), false, &links);
        assert_eq!(
            with_text,
            MessageSummary::Text {
                text: "给你".to_owned()
            }
        );
        let only_link = summary(MessageBody::Text(link), false, &links);
        assert_eq!(only_link, MessageSummary::Files);
    }

    /// 行为：消息正文里的链接带着类别进 ViewModel，插件地址认成插件页。
    #[test]
    fn message_links_carry_their_kind() {
        let targets = crate::chat::links::test_targets(&[(
            "app.grafana",
            "http://app-grafana.home.example:18480/",
        )]);
        let content = message_content(
            MessageBody::Text(
                "看 http://app-grafana.home.example:18480/ 和 example.com，打 +8613800138000。"
                    .into(),
            ),
            false,
            &targets,
        );
        let MessageContent::Text { blocks, .. } = content else {
            panic!("文字还是文字");
        };
        let found: Vec<(String, RichLinkKind)> = blocks
            .iter()
            .flat_map(|block| &block.spans)
            .filter_map(|span| {
                span.link
                    .as_ref()
                    .map(|link| (link.url.clone(), link.kind.clone()))
            })
            .collect();
        assert_eq!(
            found,
            [
                (
                    "http://app-grafana.home.example:18480/".to_owned(),
                    RichLinkKind::Plugin {
                        plugin_id: "app.grafana".to_owned()
                    }
                ),
                ("http://example.com/".to_owned(), RichLinkKind::External),
                ("tel:+8613800138000".to_owned(), RichLinkKind::Phone),
            ]
        );
    }
}
