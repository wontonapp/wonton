//! 将默认 Bot 的原始事实投影到当前请求者的插件和引导响应。

use axum::Extension;
use axum::Json;
use axum::response::{IntoResponse, Response};
use hyper::{HeaderMap, StatusCode};
use ruma_common::UserId;
use wonton_protocol::{
    BotChatAvailability, BotKeeper, BotRef, ContactState, DefaultBotVisibility, InstalledPlugin,
    MemberStatus, Onboarding, PluginDetail,
};

use super::api::{self, error_response};
use super::bot_visibility::{
    CallerFacts, PersonContactFacts, bot_identity_and_contact_facts, chat_availability, visibility,
};
use super::{Gateway, Person, TunnelPeer};
use crate::contacts::PersonContactRecordRead;
use crate::plugin::PluginId;
use crate::plugin::contacts::BotListing;
use crate::plugin::store::StoreError;

#[cfg(test)]
#[path = "bot_projection_test_gate.rs"]
pub(super) mod bot_projection_test_gate;

pub(super) fn identity(
    gateway: &Gateway,
    peer: Option<TunnelPeer>,
    headers: &HeaderMap,
    expected: &Person,
    id: &PluginId,
) -> Result<(Option<String>, DefaultBotVisibility), Box<Response>> {
    let bot = BotRef {
        plugin_id: id.to_string(),
        bot: "default".to_owned(),
    };
    let facts =
        bot_identity_and_contact_facts(gateway, peer, headers, &bot).map_err(store_error)?;
    #[cfg(test)]
    bot_projection_test_gate::after_initial_read();
    let CallerFacts::Known {
        person,
        user_id: Some(user_id),
        ..
    } = &facts.caller
    else {
        return Ok((None, DefaultBotVisibility::Unknown));
    };
    let Ok(caller_user_id) = <&UserId>::try_from(user_id.as_str()) else {
        return Ok((None, DefaultBotVisibility::Unknown));
    };
    if person != expected {
        return Ok((None, DefaultBotVisibility::Unknown));
    }
    let role = if person.is_owner() {
        None
    } else {
        let Some(member) = gateway.members.get(&person.username) else {
            return Ok((None, DefaultBotVisibility::Unknown));
        };
        if member.status != MemberStatus::Joined || member.user_id.as_str() != user_id.as_str() {
            return Ok((None, DefaultBotVisibility::Unknown));
        }
        let Some(role) = gateway.roles.get(&member.role_id) else {
            return Ok((None, DefaultBotVisibility::Unknown));
        };
        Some((member, role))
    };
    let has_record = matches!(
        &facts.contact,
        PersonContactFacts::Record(PersonContactRecordRead::Person { .. })
    );
    let Some(record) = facts.bot.as_ref() else {
        return Ok((
            None,
            if has_record {
                DefaultBotVisibility::Absent
            } else {
                DefaultBotVisibility::Unknown
            },
        ));
    };
    let availability = chat_availability(gateway, &facts);
    // 不能用也给出身份：插件页照样点得开和它的私聊，房间行带着原因。
    if availability != BotChatAvailability::Available {
        return Ok((
            Some(record.listing.user_id.clone()),
            visibility(availability),
        ));
    }
    let own = matches!(&record.listing.keeper, BotKeeper::User { username } if username == &person.username);
    let shared = record.listing.keeper == BotKeeper::Household
        && (person.is_owner()
            || role
                .as_ref()
                .is_some_and(|(_, role)| role.permissions.shared_bots));
    let Some(confirmed) = confirm_visible(
        gateway,
        peer,
        headers,
        &bot,
        &facts.caller,
        &record.listing,
        !own && !shared,
    )?
    else {
        return Ok((None, DefaultBotVisibility::Unknown));
    };
    if let Some((member, previous_role)) = role
        && (gateway.members.get(&person.username).as_ref() != Some(&member)
            || gateway.roles.get(&member.role_id).as_ref() != Some(&previous_role))
    {
        return Ok((None, DefaultBotVisibility::Unknown));
    }
    if let Some(denied) = api::plugin_denied(gateway, peer.map(Extension), headers) {
        return Err(Box::new(denied));
    }
    api::current_requester(gateway, peer.map(Extension), headers, expected)?;
    visible_identity(confirmed.user_id, caller_user_id)
}

fn visible_identity(
    user_id: String,
    caller_user_id: &UserId,
) -> Result<(Option<String>, DefaultBotVisibility), Box<Response>> {
    let bot_user_id = <&UserId>::try_from(user_id.as_str()).map_err(|error| {
        Box::new(error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "invalid_bot_record",
            &error.to_string(),
        ))
    })?;
    if bot_user_id.server_name() != caller_user_id.server_name() {
        return Err(Box::new(error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "invalid_bot_record",
            "Bot identity is not on the caller's homeserver",
        )));
    }
    Ok((Some(user_id), DefaultBotVisibility::Visible))
}

fn confirm_visible(
    gateway: &Gateway,
    peer: Option<TunnelPeer>,
    headers: &HeaderMap,
    bot: &BotRef,
    caller: &CallerFacts,
    listing: &BotListing,
    requires_contact: bool,
) -> Result<Option<BotListing>, Box<Response>> {
    let current =
        bot_identity_and_contact_facts(gateway, peer, headers, bot).map_err(store_error)?;
    if &current.caller != caller {
        return Ok(None);
    }
    let Some(record) = current.bot else {
        return Ok(None);
    };
    if &record.listing != listing {
        return Ok(None);
    }
    if requires_contact
        && !matches!(
            current.contact,
            PersonContactFacts::Record(PersonContactRecordRead::Person {
                state: ContactState::Active,
                ..
            })
        )
    {
        return Ok(None);
    }
    Ok(Some(record.listing))
}

pub(super) fn installed(
    gateway: &Gateway,
    peer: Option<TunnelPeer>,
    headers: &HeaderMap,
    expected: &Person,
    plugin: &mut InstalledPlugin,
) -> Result<(), Box<Response>> {
    let id = plugin_id(&plugin.id)?;
    let (user_id, visibility) = identity(gateway, peer, headers, expected, &id)?;
    plugin.bot_user_id = user_id;
    plugin.default_bot_visibility = Some(visibility);
    Ok(())
}

pub(super) fn installed_response(
    gateway: &Gateway,
    peer: Option<TunnelPeer>,
    headers: &HeaderMap,
    expected: &Person,
    mut plugin: InstalledPlugin,
    status: StatusCode,
) -> Response {
    match installed(gateway, peer, headers, expected, &mut plugin) {
        Ok(()) => (status, Json(plugin)).into_response(),
        Err(response) => *response,
    }
}

pub(super) fn detail(
    gateway: &Gateway,
    peer: Option<TunnelPeer>,
    headers: &HeaderMap,
    expected: &Person,
    detail: &mut PluginDetail,
) -> Result<(), Box<Response>> {
    if let Some(plugin) = &mut detail.installed {
        installed(gateway, peer, headers, expected, plugin)?;
    }
    Ok(())
}

pub(super) fn detail_response(
    gateway: &Gateway,
    peer: Option<TunnelPeer>,
    headers: &HeaderMap,
    expected: &Person,
    mut value: PluginDetail,
) -> Response {
    match detail(gateway, peer, headers, expected, &mut value) {
        Ok(()) => Json(value).into_response(),
        Err(response) => *response,
    }
}

pub(super) fn onboarding(
    gateway: &Gateway,
    peer: Option<TunnelPeer>,
    headers: &HeaderMap,
    expected: &Person,
    view: &mut Onboarding,
) -> Result<(), Box<Response>> {
    let Some(default) = view.default_bot_plugin.as_mut() else {
        return Ok(());
    };
    let id = plugin_id(&default.id)?;
    let (user_id, visibility) = if gateway.plugins.is_installed(&id) {
        identity(gateway, peer, headers, expected, &id)?
    } else {
        (None, DefaultBotVisibility::Absent)
    };
    default.bot_user_id = user_id;
    default.visibility = Some(visibility);
    Ok(())
}

fn plugin_id(value: &str) -> Result<PluginId, Box<Response>> {
    PluginId::parse(value).map_err(|error| {
        Box::new(error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "invalid_plugin_record",
            &error.to_string(),
        ))
    })
}

pub(super) fn store_error(error: StoreError) -> Box<Response> {
    let (status, kind) = match &error {
        StoreError::Io { .. } => (StatusCode::SERVICE_UNAVAILABLE, "bot_record_unavailable"),
        StoreError::Corrupt { .. } => (StatusCode::INTERNAL_SERVER_ERROR, "bot_record_corrupt"),
    };
    Box::new(error_response(status, kind, &error.to_string()))
}
