//! 时间线条目：和 matrix-sdk-ui 的时间线一一对应，下标不变，增量可以原样转给界面。

use std::sync::Arc;

use matrix_sdk::ruma::{
    OwnedUserId,
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
    /// 成员保持加入状态时修改名字或头像；名字变化才有旧名。
    ProfileChange {
        user_id: OwnedUserId,
        name: String,
        previous_name: Option<String>,
        avatar_changed: bool,
    },
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

/// 状态事件只显示房间改名、成员离开或被封禁，以及加入期间的名字和头像变化。
pub(crate) fn shown(event: &AnySyncTimelineEvent, rules: &RoomVersionRules) -> bool {
    default_event_filter(event, rules)
        && match event {
            AnySyncTimelineEvent::MessageLike(_) => true,
            AnySyncTimelineEvent::State(AnySyncStateEvent::RoomName(_)) => true,
            AnySyncTimelineEvent::State(AnySyncStateEvent::RoomMember(member)) => {
                matches!(
                    member.membership(),
                    MembershipState::Leave | MembershipState::Ban
                ) || member
                    .as_original()
                    .is_some_and(crate::profile::member_profile_changed)
            }
            AnySyncTimelineEvent::State(_) => false,
        }
}

fn event_kind(event: &EventTimelineItem, names: &MemberNames) -> EntryKind {
    if let Some(message) = message(event, names) {
        return EntryKind::Message(Box::new(message));
    }
    if let TimelineItemContent::ProfileChange(change) = event.content() {
        let user_id = change.user_id().to_owned();
        let name_change = change.displayname_change();
        let name = name_change.map_or_else(
            || names.name(&user_id),
            |change| change.new.clone().unwrap_or_else(|| user_id.to_string()),
        );
        let previous_name =
            name_change.map(|change| change.old.clone().unwrap_or_else(|| user_id.to_string()));
        return EntryKind::ProfileChange {
            user_id,
            name,
            previous_name,
            avatar_changed: change.avatar_url_change().is_some(),
        };
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
        // `event_kind` 已经处理资料变化。
        TimelineItemContent::ProfileChange(_) => unreachable!("资料变化有独立的条目种类"),
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn member_event(previous: Option<Value>, content: Value) -> AnySyncTimelineEvent {
        let mut event = json!({
            "type": "m.room.member",
            "event_id": "$member:example.org",
            "sender": "@bot:example.org",
            "state_key": "@bot:example.org",
            "origin_server_ts": 1,
            "content": content,
        });
        if let Some(previous) = previous {
            event["unsigned"] = json!({ "prev_content": previous });
        }
        serde_json::from_value(event).expect("有效成员事件")
    }

    #[test]
    fn joined_members_name_and_avatar_changes_are_shown() {
        let previous = json!({"membership": "join", "displayname": "Bot"});
        for content in [
            json!({"membership": "join", "displayname": "Renamed"}),
            json!({"membership": "join"}),
            json!({"membership": "join", "displayname": "Bot", "avatar_url": "mxc://example.org/a"}),
        ] {
            let event = member_event(Some(previous.clone()), content);
            assert!(shown(&event, &RoomVersionRules::V11));
        }
        let previous = json!({"membership": "join", "avatar_url": "mxc://example.org/a"});
        let event = member_event(Some(previous), json!({"membership": "join"}));
        assert!(shown(&event, &RoomVersionRules::V11));
    }

    #[test]
    fn reason_only_and_non_join_profile_updates_are_hidden() {
        let joined = json!({"membership": "join", "displayname": "Bot"});
        let reason_only = json!({"membership": "join", "displayname": "Bot", "reason": "updated"});
        assert!(!shown(
            &member_event(Some(joined.clone()), reason_only),
            &RoomVersionRules::V11,
        ));
        assert!(!shown(&member_event(None, joined), &RoomVersionRules::V11));
        for membership in ["leave", "invite", "knock"] {
            let previous = json!({"membership": membership, "displayname": "Bot"});
            let content = json!({"membership": "join", "displayname": "Renamed"});
            assert!(!shown(
                &member_event(Some(previous), content),
                &RoomVersionRules::V11,
            ));
        }
    }

    #[test]
    fn leaving_and_banned_members_remain_visible() {
        for membership in ["leave", "ban"] {
            let event = member_event(None, json!({"membership": membership}));
            assert!(shown(&event, &RoomVersionRules::V11));
        }
    }
}
