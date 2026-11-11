//! App 读和改 Bot 钉的页面（ADR-0052）：列表、移除、标已看、收回往主面板钉页的同意。
//!
//! 页面由 Bot 经 MCP 总线钉，存在服务器上；App 读出来和用户自己放的合在一起显示。

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use hyper::{HeaderMap, StatusCode};
use wonton_protocol::{BotPanelPin, BotPanelPins, BotPinConsent, BotPinOwner};

use super::api::error_response;
use super::{Gateway, TunnelPeer, isolation};
use crate::plugin::PluginId;

fn unauthorized(
    gateway: &Gateway,
    peer: Option<Extension<TunnelPeer>>,
    headers: &HeaderMap,
) -> Option<Response> {
    if gateway
        .device_user(peer.map(|Extension(peer)| peer))
        .is_none()
        && !super::api::has_owner_token(gateway, headers)
    {
        return Some(error_response(
            StatusCode::UNAUTHORIZED,
            "unauthenticated",
            "这台设备没有配对或者已被移除，要重新配对",
        ));
    }
    if !isolation::not_from_another_page(headers, &gateway.core_origin()) {
        return Some(error_response(
            StatusCode::FORBIDDEN,
            "cross_site",
            "面板接口不接受其他网页发来的请求",
        ));
    }
    None
}

fn failed(error: impl std::fmt::Display) -> Response {
    error_response(
        StatusCode::INTERNAL_SERVER_ERROR,
        "storage",
        &error.to_string(),
    )
}

fn view(
    gateway: &Gateway,
    id: &PluginId,
    bot: &str,
    pin: crate::plugin::pins::PanelPin,
) -> BotPanelPin {
    let name = gateway
        .plugins
        .bot_name(id, bot)
        .map(|name| crate::mcp::card_text(&name))
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| bot.to_owned());
    BotPanelPin {
        id: pin.id,
        scope: pin.scope,
        tab: pin.tab,
        title: pin.title,
        attention: pin.attention,
        plugin_id: id.as_str().to_owned(),
        bot: bot.to_owned(),
        name,
    }
}

pub(super) async fn list(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
) -> Response {
    if let Some(response) = unauthorized(&gateway, peer, &headers) {
        return response;
    }
    let all = match gateway.plugins.store().pins().all() {
        Ok(all) => all,
        Err(e) => return failed(e),
    };
    let mut pins = Vec::new();
    for (id, record) in all {
        for (bot, entry) in record.bots {
            pins.extend(
                entry
                    .pins
                    .into_iter()
                    .map(|pin| view(&gateway, &id, &bot, pin)),
            );
        }
    }
    let consents = match gateway.plugins.store().pins().all() {
        Ok(all) => all,
        Err(e) => return failed(e),
    }
    .into_iter()
    .flat_map(|(id, record)| {
        record
            .bots
            .into_iter()
            .filter(|(_, entry)| entry.main_consent || !entry.pins.is_empty())
            .map(move |(bot, entry)| (id.clone(), bot, entry.main_consent))
    })
    .map(|(id, bot, main_consent)| {
        let name = gateway
            .plugins
            .bot_name(&id, &bot)
            .map(|name| crate::mcp::card_text(&name))
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| bot.clone());
        BotPinOwner {
            plugin_id: id.as_str().to_owned(),
            bot,
            name,
            main_consent,
        }
    })
    .collect();
    Json(BotPanelPins { pins, consents }).into_response()
}

pub(super) async fn remove(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path(pin): Path<String>,
) -> Response {
    if let Some(response) = unauthorized(&gateway, peer, &headers) {
        return response;
    }
    let found = match gateway.plugins.store().pins().find(&pin) {
        Ok(found) => found,
        Err(e) => return failed(e),
    };
    let Some((plugin, bot, _)) = found else {
        return error_response(StatusCode::NOT_FOUND, "not_found", "没有这个钉页");
    };
    let result = gateway.plugins.store().pins().with(&plugin, |record| {
        record.bots.get_mut(&bot).is_some_and(|entry| {
            let before = entry.pins.len();
            entry.pins.retain(|candidate| candidate.id != pin);
            entry.pins.len() != before
        })
    });
    match result {
        Ok(true) => Json(serde_json::json!({"removed": true})).into_response(),
        Ok(false) => error_response(StatusCode::NOT_FOUND, "not_found", "没有这个钉页"),
        Err(e) => failed(e),
    }
}

pub(super) async fn seen(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path(pin): Path<String>,
) -> Response {
    if let Some(response) = unauthorized(&gateway, peer, &headers) {
        return response;
    }
    let found = match gateway.plugins.store().pins().find(&pin) {
        Ok(found) => found,
        Err(e) => return failed(e),
    };
    let Some((plugin, bot, _)) = found else {
        return error_response(StatusCode::NOT_FOUND, "not_found", "没有这个钉页");
    };
    let result = gateway.plugins.store().pins().with(&plugin, |record| {
        let Some(found) = record
            .bots
            .get_mut(&bot)
            .and_then(|entry| entry.pins.iter_mut().find(|candidate| candidate.id == pin))
        else {
            return false;
        };
        found.attention = false;
        true
    });
    match result {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => error_response(StatusCode::NOT_FOUND, "not_found", "没有这个钉页"),
        Err(e) => failed(e),
    }
}

pub(super) async fn consent(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path((plugin, bot)): Path<(String, String)>,
    Json(body): Json<BotPinConsent>,
) -> Response {
    if let Some(response) = unauthorized(&gateway, peer, &headers) {
        return response;
    }
    let Ok(plugin) = PluginId::parse(&plugin) else {
        return error_response(StatusCode::BAD_REQUEST, "invalid_input", "插件 ID 不对");
    };
    match gateway.plugins.store().pins().with(&plugin, |record| {
        record.bots.entry(bot).or_default().main_consent = body.allowed;
    }) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => failed(e),
    }
}
