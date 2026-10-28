//! 时间线条目：和 matrix-sdk-ui 的时间线一一对应，下标不变，增量可以原样转给界面。

use std::sync::Arc;

use matrix_sdk::ruma::events::{StateEventContentChange, room::name::RoomNameEventContent};
use matrix_sdk_ui::timeline::{
    AnyOtherStateEventContentChange, EventTimelineItem, MembershipChange, TimelineItem,
    TimelineItemContent, TimelineItemKind, VirtualTimelineItem,
};

use crate::message::{MemberNames, Message, message, sender_name};

/// 时间线里的一条。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimelineEntry {
    /// 在这条时间线里稳定的标识：本地回显发出后、内容被编辑后都不变。
    pub id: String,
    pub kind: EntryKind,
}

/// 条目的种类。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EntryKind {
    /// 消息比其他种类大得多，装箱放着。
    Message(Box<Message>),
    /// 一行说明：成员变化、改名、房间设置的变化等。
    Notice(String),
    /// 日期分隔线，值是这一天里第一条消息的时间（Unix 毫秒）。
    DateDivider(u64),
    /// 本账号的已读位置。
    ReadMarker,
    /// 房间的开头。
    TimelineStart,
}

pub(crate) fn entry(item: &Arc<TimelineItem>, names: &MemberNames) -> TimelineEntry {
    let kind = match item.kind() {
        TimelineItemKind::Event(event) => event_kind(event, names),
        TimelineItemKind::Virtual(VirtualTimelineItem::DateDivider(timestamp)) => {
            EntryKind::DateDivider(timestamp.get().into())
        }
        TimelineItemKind::Virtual(VirtualTimelineItem::ReadMarker) => EntryKind::ReadMarker,
        TimelineItemKind::Virtual(VirtualTimelineItem::TimelineStart) => EntryKind::TimelineStart,
    };
    TimelineEntry {
        id: item.unique_id().0.clone(),
        kind,
    }
}

fn event_kind(event: &EventTimelineItem, names: &MemberNames) -> EntryKind {
    if let Some(message) = message(event, names) {
        return EntryKind::Message(Box::new(message));
    }
    EntryKind::Notice(notice(event))
}

/// 非消息事件的一行说明。
fn notice(event: &EventTimelineItem) -> String {
    let who = sender_name(event);
    match event.content() {
        TimelineItemContent::MembershipChange(change) => {
            membership_notice(change.user_id().as_str(), change.change())
        }
        TimelineItemContent::ProfileChange(change) => match change.displayname_change() {
            Some(name) => format!(
                "{} 改名为 {}",
                name.old.as_deref().unwrap_or(change.user_id().as_str()),
                name.new.as_deref().unwrap_or(change.user_id().as_str())
            ),
            None => format!("{who} 更新了头像"),
        },
        TimelineItemContent::OtherState(state) => match state.content() {
            AnyOtherStateEventContentChange::RoomName(StateEventContentChange::Original {
                content: RoomNameEventContent { name, .. },
                ..
            }) => format!("{who} 把房间名改为 {name}"),
            AnyOtherStateEventContentChange::RoomEncryption(_) => {
                format!("{who} 开启了端到端加密")
            }
            AnyOtherStateEventContentChange::RoomCreate(_) => format!("{who} 创建了房间"),
            other => format!("{who} 修改了房间设置（{}）", other.event_type()),
        },
        TimelineItemContent::CallInvite | TimelineItemContent::RtcNotification { .. } => {
            format!("{who} 发起了通话")
        }
        TimelineItemContent::FailedToParseMessageLike { event_type, .. } => {
            format!("无法解析的消息（{event_type}）")
        }
        TimelineItemContent::FailedToParseState { event_type, .. } => {
            format!("无法解析的房间事件（{event_type}）")
        }
        // 消息类事件已经由 `message` 处理。
        TimelineItemContent::MsgLike(_) => format!("{who} 发了一条消息"),
    }
}

fn membership_notice(user: &str, change: Option<MembershipChange>) -> String {
    let action = match change {
        Some(MembershipChange::Joined | MembershipChange::InvitationAccepted) => "加入了房间",
        Some(MembershipChange::Left) => "离开了房间",
        Some(MembershipChange::Invited) => "被邀请加入房间",
        Some(MembershipChange::InvitationRejected) => "拒绝了邀请",
        Some(MembershipChange::InvitationRevoked) => "的邀请被撤回",
        Some(MembershipChange::Kicked) => "被移出房间",
        Some(MembershipChange::Banned | MembershipChange::KickedAndBanned) => "被封禁",
        Some(MembershipChange::Unbanned) => "被解除封禁",
        Some(
            MembershipChange::Knocked
            | MembershipChange::KnockAccepted
            | MembershipChange::KnockRetracted
            | MembershipChange::KnockDenied,
        ) => "申请加入房间",
        Some(
            MembershipChange::None | MembershipChange::Error | MembershipChange::NotImplemented,
        )
        | None => "的成员状态有变化",
    };
    format!("{user} {action}")
}
