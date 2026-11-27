//! 供响应投影读取的分域事实，不是授权结论或跨来源的原子快照。
//! 消费者仍须守住原请求的身份、origin 和既有权限；这些值不证明等待期间没有 ABA。

use hyper::HeaderMap;
use ruma_common::UserId;
use wonton_protocol::{
    BotChatAvailability, BotChatUnavailable, BotKeeper, BotRef, ContactState, DefaultBotVisibility,
    MemberStatus,
};

use super::{Gateway, Person, TunnelPeer, api, isolation};
use crate::contacts::PersonContactRecordRead;
use crate::plugin::contacts::BotIdentityRecordFacts;
use crate::plugin::store::StoreError;

#[derive(Debug, PartialEq, Eq)]
pub(super) struct BotIdentityAndContactFacts {
    pub(super) bot: Option<BotIdentityRecordFacts>,
    pub(super) caller: CallerFacts,
    pub(super) contact: PersonContactFacts,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum CallerFacts {
    Unknown,
    Known {
        source: CallerSource,
        person: Person,
        /// 未知时不按公开访问域名拼出身份。
        user_id: Option<String>,
    },
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum CallerSource {
    OwnerBearer,
    PairedDevice,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum PersonContactFacts {
    NotConnected,
    UnknownIdentity,
    Record(PersonContactRecordRead),
}

/// 按原请求的当前认证来源读身份；无已证身份时未知，不从无 peer 推主人。
/// Person 行的 room_id 是独立事实，不以 Bot 主房间或旁聊房间补齐。
pub(super) fn bot_identity_and_contact_facts(
    gateway: &Gateway,
    peer: Option<TunnelPeer>,
    headers: &HeaderMap,
    bot: &BotRef,
) -> Result<BotIdentityAndContactFacts, StoreError> {
    let bot_record = gateway.plugins.bot_identity_record(bot)?;
    let caller = caller_facts(gateway, peer, headers);
    let contact = match (&gateway.contacts, &caller) {
        (None, _) => PersonContactFacts::NotConnected,
        (
            Some(contacts),
            CallerFacts::Known {
                user_id: Some(user_id),
                ..
            },
        ) => PersonContactFacts::Record(contacts.person_contact_record(bot, user_id)),
        (Some(_), _) => PersonContactFacts::UnknownIdentity,
    };
    Ok(BotIdentityAndContactFacts {
        bot: bot_record,
        caller,
        contact,
    })
}

/// 按原始事实判断当前请求者能不能和这个 Bot 聊天：本人的、共享 Bot（主人或角色允许）和已成立的联系人能用；
/// 别人的私人 Bot 分“没授权”和“授权已撤”：从来没有过记录（缺失、等对方同意）算没授权，拒绝过、对方已不存在
/// 这类有往来的算授权已撤；读不明时未知。角色和成员记录要和调用人的当前身份一致，不一致只说明快照变了，
/// 不当作能用。
pub(super) fn chat_availability(
    gateway: &Gateway,
    facts: &BotIdentityAndContactFacts,
) -> BotChatAvailability {
    let CallerFacts::Known {
        person,
        user_id: Some(user_id),
        ..
    } = &facts.caller
    else {
        return BotChatAvailability::Unknown;
    };
    if <&UserId>::try_from(user_id.as_str()).is_err() {
        return BotChatAvailability::Unknown;
    }
    let shared_bots = if person.is_owner() {
        true
    } else {
        let Some(member) = gateway.members.get(&person.username) else {
            return BotChatAvailability::Unknown;
        };
        if member.status != MemberStatus::Joined || member.user_id.as_str() != user_id.as_str() {
            return BotChatAvailability::Unknown;
        }
        let Some(role) = gateway.roles.get(&member.role_id) else {
            return BotChatAvailability::Unknown;
        };
        role.permissions.shared_bots
    };
    let (contact_active, grant_revoked) = match &facts.contact {
        PersonContactFacts::Record(PersonContactRecordRead::Person {
            state: ContactState::Active,
            ..
        }) => (true, false),
        PersonContactFacts::Record(PersonContactRecordRead::Person {
            state: ContactState::Declined | ContactState::NotFound,
            ..
        }) => (false, true),
        _ => (false, false),
    };
    let Some(record) = facts.bot.as_ref() else {
        return BotChatAvailability::Unknown;
    };
    let own = matches!(&record.listing.keeper, BotKeeper::User { username } if username == &person.username);
    let shared = record.listing.keeper == BotKeeper::Household;
    if own || shared && shared_bots || contact_active {
        return BotChatAvailability::Available;
    }
    match record.listing.keeper {
        BotKeeper::User { .. } if grant_revoked => BotChatAvailability::Unavailable {
            reason: BotChatUnavailable::GrantNotActive,
        },
        BotKeeper::User { .. } => BotChatAvailability::Unavailable {
            reason: BotChatUnavailable::OwnerPrivate,
        },
        // 共享 Bot 但角色不许共享，或归属方式还不认识：确定不能用。
        _ => BotChatAvailability::Unavailable {
            reason: BotChatUnavailable::Other,
        },
    }
}

/// 同 [`chat_availability`]，但重新读一次事实；调用人已经变了（换人、注销、设备被撤）时未知。
pub(super) fn chat_availability_of(
    gateway: &Gateway,
    peer: Option<TunnelPeer>,
    headers: &HeaderMap,
    bot: &BotRef,
    caller: &CallerFacts,
) -> Result<BotChatAvailability, StoreError> {
    let facts = bot_identity_and_contact_facts(gateway, peer, headers, bot)?;
    if &facts.caller != caller {
        return Ok(BotChatAvailability::Unknown);
    }
    Ok(chat_availability(gateway, &facts))
}

/// 按归属和联系人给默认 Bot 的身份可见性：能用才披露身份；确定没有授权时按没有，其余未知。
pub(super) fn visibility(availability: BotChatAvailability) -> DefaultBotVisibility {
    match availability {
        BotChatAvailability::Available => DefaultBotVisibility::Visible,
        BotChatAvailability::Unavailable { .. } => DefaultBotVisibility::Absent,
        BotChatAvailability::Unknown => DefaultBotVisibility::Unknown,
    }
}

pub(super) fn caller_facts(
    gateway: &Gateway,
    peer: Option<TunnelPeer>,
    headers: &HeaderMap,
) -> CallerFacts {
    // requester 会记语言；事实读取只复用它的纯令牌校验，不触发写入。
    let (person, source) = if api::has_owner_token(gateway, headers) {
        (
            Person::owner(gateway.owner.username()),
            CallerSource::OwnerBearer,
        )
    } else {
        let Some(person) = gateway.device_person(peer) else {
            return CallerFacts::Unknown;
        };
        if !isolation::not_from_another_page(headers, &gateway.core_origin()) {
            return CallerFacts::Unknown;
        }
        (person, CallerSource::PairedDevice)
    };
    let user_id = if person.is_owner() {
        gateway
            .matrix
            .as_ref()
            .map(|matrix| matrix.user_id(&person.username))
    } else {
        member_user_id(gateway, &person)
    };
    CallerFacts::Known {
        source,
        person,
        user_id,
    }
}

fn member_user_id(gateway: &Gateway, person: &Person) -> Option<String> {
    let record = gateway.members.get(&person.username)?;
    if record.status == MemberStatus::Removed {
        return None;
    }
    let user_id = <&UserId>::try_from(record.user_id.as_str()).ok()?;
    if user_id.localpart() != person.username
        || gateway
            .matrix
            .as_ref()
            .is_some_and(|matrix| user_id.server_name().as_str() != matrix.server_name())
    {
        return None;
    }
    Some(record.user_id)
}

#[cfg(test)]
#[path = "bot_visibility_tests.rs"]
mod bot_visibility_tests;
