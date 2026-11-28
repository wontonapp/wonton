//! 房间成员的显示名、头像和本账号在房间里的撤回权限。

use std::collections::HashMap;

use matrix_sdk::ruma::{OwnedUserId, UserId};
use matrix_sdk_ui::timeline::{EventTimelineItem, TimelineDetails};

/// 房间成员的显示名（显示发送者、“谁已读”和引用的发送者），以及本账号在房间里的撤回权限。没有记录的成员显示 [`fallback_name`]。
///
/// 两样都跟着房间状态变，一起放在这里，变了的时候时间线按 [`Self::affects`] 重发受影响的条目。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct MemberNames {
    names: HashMap<OwnedUserId, String>,
    avatars: HashMap<OwnedUserId, String>,
    redaction: RedactionPower,
    own: Option<OwnedUserId>,
    direct: bool,
}

/// 本账号在房间里能撤回谁的消息（Matrix 的 power level）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct RedactionPower {
    pub(crate) own: bool,
    pub(crate) others: bool,
}

impl MemberNames {
    pub(crate) fn new(names: HashMap<OwnedUserId, String>, own: OwnedUserId, direct: bool) -> Self {
        Self {
            names,
            avatars: HashMap::new(),
            redaction: RedactionPower::default(),
            own: Some(own),
            direct,
        }
    }

    pub(crate) fn with_redaction(mut self, redaction: RedactionPower) -> Self {
        self.redaction = redaction;
        self
    }

    pub(crate) fn with_avatars(mut self, avatars: HashMap<OwnedUserId, String>) -> Self {
        self.avatars = avatars;
        self
    }

    pub(crate) fn own(&self) -> Option<&UserId> {
        self.own.as_deref()
    }

    pub(crate) fn direct(&self) -> bool {
        self.direct
    }

    /// 在场成员的用户 ID；"谁在群里"按它算，不按显示名。
    pub(crate) fn user_ids(&self) -> Vec<String> {
        self.names.keys().map(|user| user.to_string()).collect()
    }

    /// 在场成员，连同给他们设的头像（`mxc://`）。
    pub(crate) fn members(&self) -> impl Iterator<Item = (&OwnedUserId, &String)> {
        self.names.iter()
    }

    pub(crate) fn contains(&self, user: &UserId) -> bool {
        self.names.contains_key(user)
    }

    pub(crate) fn is_own(&self, user: &UserId) -> bool {
        self.own.as_deref() == Some(user)
    }

    pub(crate) fn name(&self, user: &UserId) -> String {
        self.get(user).to_owned()
    }

    pub(crate) fn avatar(&self, user: &UserId) -> Option<&str> {
        self.avatars.get(user).map(String::as_str)
    }

    fn get<'a>(&'a self, user: &'a UserId) -> &'a str {
        self.names
            .get(user)
            .map(String::as_str)
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| fallback_name(user))
    }

    pub(super) fn can_redact(&self, event: &EventTimelineItem) -> bool {
        if event.is_own() {
            self.redaction.own
        } else {
            self.redaction.others
        }
    }

    pub(crate) fn affects(&self, event: &EventTimelineItem, next: &Self) -> bool {
        let quoted_sender = event
            .content()
            .in_reply_to()
            .and_then(|reply| match reply.event {
                TimelineDetails::Ready(quoted) => Some(quoted.sender),
                _ => None,
            });
        self.redaction != next.redaction
            || self.direct != next.direct
            || self.get(event.sender()) != next.get(event.sender())
            || self.avatar(event.sender()) != next.avatar(event.sender())
            || quoted_sender.is_some_and(|user| {
                self.get(&user) != next.get(&user) || self.avatar(&user) != next.avatar(&user)
            })
            || event
                .read_receipts()
                .keys()
                .any(|user| self.get(user) != next.get(user))
    }
}

/// 没有显示名的人显示账号名（用户 ID 里 `@` 和 `:` 之间的部分）：家人看不懂完整的用户 ID。和 SDK 的
/// `RoomMember::name` 一致，成员资料读得到和读不到时显示的是同一个。
pub(crate) fn fallback_name(user: &UserId) -> &str {
    user.localpart()
}
