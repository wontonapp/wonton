//! 时间线条目：和 matrix-sdk-ui 的时间线一一对应，下标不变，增量可以原样转给界面。

use std::sync::Arc;

use matrix_sdk::ruma::{
    events::{
        AnySyncStateEvent, AnySyncTimelineEvent, StateEventContentChange,
        room::{member::MembershipState, name::RoomNameEventContent},
    },
    room_version_rules::RoomVersionRules,
};
use matrix_sdk_ui::timeline::{
    AnyOtherStateEventContentChange, EventTimelineItem, MembershipChange, TimelineItem,
    TimelineItemContent, TimelineItemKind, VirtualTimelineItem, default_event_filter,
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
    /// 一行说明：成员离开、房间改名、通话等。
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

/// 时间线里显示哪些事件：SDK 的默认规则之外，房间状态事件只留聊天的人要知道的两种：房间改名，和成员离开或被
/// 封禁（对方走了，再发消息就没人收了）。其余的（建房、加密、权限、谁加入、改昵称）新房间一打开就是一串，对聊天
/// 没有意义。
pub(crate) fn shown(event: &AnySyncTimelineEvent, rules: &RoomVersionRules) -> bool {
    default_event_filter(event, rules)
        && match event {
            AnySyncTimelineEvent::MessageLike(_) => true,
            AnySyncTimelineEvent::State(AnySyncStateEvent::RoomName(_)) => true,
            AnySyncTimelineEvent::State(AnySyncStateEvent::RoomMember(member)) => matches!(
                member.membership(),
                MembershipState::Leave | MembershipState::Ban
            ),
            AnySyncTimelineEvent::State(_) => false,
        }
}

fn event_kind(event: &EventTimelineItem, names: &MemberNames) -> EntryKind {
    if let Some(message) = message(event, names) {
        return EntryKind::Message(Box::new(message));
    }
    EntryKind::Notice(notice(event, names))
}

/// 非消息事件的一行说明。
fn notice(event: &EventTimelineItem, names: &MemberNames) -> String {
    let who = sender_name(event, names);
    match event.content() {
        TimelineItemContent::MembershipChange(change) => {
            // 成员事件自己带着显示名（加入时设好的），没有时用本机已有的成员显示名。
            let user = change
                .display_name()
                .unwrap_or_else(|| names.name(change.user_id()));
            membership_notice(&user, change.change())
        }
        // `shown` 不放行资料变化，这一行只为穷尽。
        TimelineItemContent::ProfileChange(_) => format!("{who} 更新了资料"),
        TimelineItemContent::OtherState(state) => match state.content() {
            AnyOtherStateEventContentChange::RoomName(StateEventContentChange::Original {
                content: RoomNameEventContent { name, .. },
                ..
            }) => format!("{who} 把房间名改为 {name}"),
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
    // `shown` 只放行变成“离开”和“封禁”的成员事件。
    let action = match change {
        Some(MembershipChange::Left) => "离开了房间",
        Some(MembershipChange::InvitationRejected) => "拒绝了邀请",
        Some(MembershipChange::InvitationRevoked) => "的邀请被撤回",
        Some(MembershipChange::Kicked) => "被移出房间",
        Some(MembershipChange::Banned | MembershipChange::KickedAndBanned) => "被封禁",
        Some(MembershipChange::Unbanned) => "被解除封禁",
        _ => "的成员状态有变化",
    };
    format!("{user} {action}")
}
