//! 本人私人 Bot 身份的只读出口；身份不代表房间或发送资格。

use std::sync::Arc;

use axum::Extension;
use axum::Json;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use hyper::{HeaderMap, StatusCode};
use ruma_common::{RoomId, UserId};
use wonton_protocol::{
    BotKeeper, BotRef, MemberStatus, OwnPrivateBotResponse, OwnPrivateBotResult,
    PrivateBotIdentity, PrivateBotRoomProblem,
};

use super::api;
use super::bot_projection::store_error;
use super::bot_visibility::{CallerFacts, bot_identity_and_contact_facts};
use super::{Gateway, Person, TunnelPeer};
use crate::plugin::PluginId;
use crate::plugin::bots::check_bot_name;
use crate::plugin::contacts::BotIdentityRecordFacts;

pub(super) async fn read(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
) -> Response {
    if let Some(denied) = api::plugin_denied(&gateway, peer, &headers) {
        return denied;
    }
    let person = match api::requester(&gateway, peer, &headers) {
        Ok(person) => person,
        Err(denied) => return *denied,
    };
    let result = result_for_person(&gateway, peer, &headers, &person);
    if let Some(denied) = api::plugin_denied(&gateway, peer, &headers) {
        return denied;
    }
    if let Err(denied) = api::current_requester(&gateway, peer, &headers, &person) {
        return *denied;
    }
    match result {
        Ok((requester_user_id, result)) => Json(OwnPrivateBotResponse {
            requester_user_id,
            result,
        })
        .into_response(),
        Err(error) => *error,
    }
}

/// 当前人的私人 Bot 事实；`Err` 是要原样返回给壳的响应。
///
/// 主人和角色里没有私人 Bot 的人是 [`OwnPrivateBotResult::NotApplicable`]：他们的默认 Agent 是插件的默认
/// Bot。有私人 Bot 但还没建好是 `pending`，不静默退到默认 Bot，否则壳会把“还没准备好”当成“就是它”。
pub(super) fn result_for_person(
    gateway: &Gateway,
    peer: Option<Extension<TunnelPeer>>,
    headers: &HeaderMap,
    person: &Person,
) -> Result<(String, OwnPrivateBotResult), Box<Response>> {
    if person.is_owner() {
        let Some(matrix) = gateway.matrix.as_ref() else {
            return Err(unavailable(
                "matrix_unavailable",
                "Matrix identity source is unavailable",
            ));
        };
        let requester_user_id = matrix.user_id(&person.username);
        let Ok(user_id) = <&UserId>::try_from(requester_user_id.as_str()) else {
            return Err(invalid(
                "member_identity_invalid",
                "Owner identity is invalid",
            ));
        };
        if user_id.localpart() != person.username
            || user_id.server_name().as_str() != matrix.server_name()
        {
            return Err(invalid(
                "member_identity_invalid",
                "Owner identity does not match its source",
            ));
        }
        return Ok((requester_user_id, OwnPrivateBotResult::NotApplicable));
    }
    let Some(snapshot) = gateway.members.private_bot_snapshot(&person.username) else {
        return Err(unavailable(
            "member_identity_unavailable",
            "Member record is unavailable",
        ));
    };
    if snapshot.status != MemberStatus::Joined || snapshot.removing {
        return Err(unavailable(
            "member_identity_unavailable",
            "Member is not available",
        ));
    }
    let Some(role) = gateway.roles.get(&snapshot.role_id) else {
        return Err(unavailable(
            "member_role_unavailable",
            "Member role is unavailable",
        ));
    };
    let Some(matrix) = gateway.matrix.as_ref() else {
        return Err(unavailable(
            "matrix_unavailable",
            "Matrix identity source is unavailable",
        ));
    };
    let Ok(member_id) = <&UserId>::try_from(snapshot.user_id.as_str()) else {
        return Err(invalid(
            "member_identity_invalid",
            "Member identity is invalid",
        ));
    };
    if member_id.localpart() != person.username
        || member_id.server_name().as_str() != matrix.server_name()
    {
        return Err(invalid(
            "member_identity_invalid",
            "Member identity does not match its source",
        ));
    }
    if !snapshot.wants_private_bot || !role.permissions.private_bot {
        return Ok((snapshot.user_id.clone(), OwnPrivateBotResult::NotApplicable));
    }
    let result = match (&snapshot.bot, &snapshot.reservation) {
        (Some(bot), Some(reservation)) if bot != reservation => Err(invalid(
            "private_bot_conflict",
            "Private Bot reference conflicts with its reservation",
        )),
        (Some(bot), _) => {
            let peer = peer.map(|Extension(peer)| peer);
            own_identity(gateway, peer, headers, person, &snapshot.user_id, bot).and_then(|first| {
                let second = own_identity(gateway, peer, headers, person, &snapshot.user_id, bot)?;
                if first != second {
                    return Err(unavailable(
                        "private_bot_changed",
                        "Private Bot identity changed",
                    ));
                }
                Ok(OwnPrivateBotResult::Ready(identity_from_record(first)))
            })
        }
        _ => Err(unavailable(
            "private_bot_pending",
            "Private Bot creation is pending",
        )),
    };
    if gateway
        .members
        .private_bot_snapshot(&person.username)
        .as_ref()
        != Some(&snapshot)
        || gateway.roles.get(&snapshot.role_id).as_ref() != Some(&role)
    {
        return Err(unavailable("private_bot_changed", "Member or role changed"));
    }
    Ok((snapshot.user_id.clone(), result?))
}

fn own_identity(
    gateway: &Gateway,
    peer: Option<TunnelPeer>,
    headers: &HeaderMap,
    expected: &Person,
    member_user_id: &str,
    bot: &BotRef,
) -> Result<BotIdentityRecordFacts, Box<Response>> {
    if PluginId::parse(&bot.plugin_id).is_err() || check_bot_name(&bot.bot).is_err() {
        return Err(invalid(
            "private_bot_ref_invalid",
            "Private Bot reference is invalid",
        ));
    }
    let facts = bot_identity_and_contact_facts(gateway, peer, headers, bot).map_err(store_error)?;
    match facts.caller {
        CallerFacts::Known {
            person,
            user_id: Some(user_id),
            ..
        } if person == *expected && user_id == member_user_id => {}
        _ => {
            return Err(unavailable(
                "member_identity_unavailable",
                "Caller identity is unavailable",
            ));
        }
    }
    let Some(record) = facts.bot else {
        return Err(unavailable(
            "private_bot_record_missing",
            "Private Bot identity record is unavailable",
        ));
    };
    match &record.listing.keeper {
        BotKeeper::User { username } if username == &expected.username => {}
        BotKeeper::Other => {
            return Err(unavailable(
                "private_bot_keeper_unknown",
                "Private Bot keeper is unknown",
            ));
        }
        _ => {
            return Err(invalid(
                "private_bot_identity_mismatch",
                "Private Bot keeper does not match",
            ));
        }
    }
    let bot_user_id = <&UserId>::try_from(record.listing.user_id.as_str()).map_err(|_| {
        invalid(
            "private_bot_identity_mismatch",
            "Private Bot identity is invalid",
        )
    })?;
    let member_id = <&UserId>::try_from(member_user_id)
        .map_err(|_| invalid("member_identity_invalid", "Member identity is invalid"))?;
    if bot_user_id.server_name() != member_id.server_name() {
        return Err(invalid(
            "private_bot_identity_mismatch",
            "Private Bot identity does not match its source",
        ));
    }
    Ok(record)
}

fn identity_from_record(record: BotIdentityRecordFacts) -> PrivateBotIdentity {
    let (home_room, room_problem) = if record.home_room.is_empty() {
        (None, Some(PrivateBotRoomProblem::Missing))
    } else if <&RoomId>::try_from(record.home_room.as_str()).is_ok() {
        (Some(record.home_room), None)
    } else {
        (None, Some(PrivateBotRoomProblem::Invalid))
    };
    PrivateBotIdentity {
        bot: record.listing.bot,
        user_id: record.listing.user_id,
        home_room,
        room_problem,
    }
}

fn unavailable(kind: &'static str, message: &str) -> Box<Response> {
    Box::new(api::error_response(
        StatusCode::SERVICE_UNAVAILABLE,
        kind,
        message,
    ))
}

fn invalid(kind: &'static str, message: &str) -> Box<Response> {
    Box::new(api::error_response(
        StatusCode::INTERNAL_SERVER_ERROR,
        kind,
        message,
    ))
}

#[cfg(test)]
#[path = "private_bot_tests.rs"]
mod private_bot_tests;
