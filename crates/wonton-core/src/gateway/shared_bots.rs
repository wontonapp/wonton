//! `GET SHARED_BOTS_PATH`：按当前请求人列出装着的 Bot 和能不能聊。
//!
//! 列表只负责发现：谁在家里装过、能不能发现、为什么不能聊。发送资格仍由房间和 Bot 的既有规则决定。
//! 不受 `open_apps` 过滤：插件没开给这个人时照样列出，原因写 [`BotChatUnavailable::AppNotShared`]。

use std::collections::HashMap;
use std::sync::Arc;

use axum::Extension;
use axum::Json;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use hyper::{HeaderMap, StatusCode};
use wonton_protocol::{BotRef, SharedBot, SharedBotsState, SharedBotsView};

use super::api::error_response;
use super::bot_visibility::{bot_access, caller_facts, caller_ready};
use super::{Gateway, TunnelPeer, isolation};
use crate::plugin::bot_services::DEFAULT_BOT;
use crate::plugin::contacts::BotListing;

pub(super) async fn get(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
) -> Response {
    let peer = peer.map(|Extension(peer)| peer);
    if gateway.device_user(peer).is_none() {
        return error_response(
            StatusCode::UNAUTHORIZED,
            "unauthenticated",
            "这台设备没有配对或者已被移除，要重新配对",
        );
    }
    if !isolation::not_from_another_page(&headers, &gateway.core_origin()) {
        return error_response(
            StatusCode::FORBIDDEN,
            "cross_site",
            "Bot 列表接口不接受其他网页发来的请求",
        );
    }

    let caller = caller_facts(&gateway, peer, &headers);
    let state = if caller_ready(&gateway, &caller) {
        SharedBotsState::Ready
    } else {
        SharedBotsState::Unknown
    };
    let langs = super::api::languages(&headers);
    let installed = gateway.plugins.installed(&langs);
    let listings = match gateway.plugins.bot_listings() {
        Ok(listings) => listings,
        Err(error) => {
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage", &error);
        }
    };
    let mut by_plugin: HashMap<String, Vec<BotListing>> = HashMap::new();
    for listing in listings {
        by_plugin
            .entry(listing.bot.plugin_id.clone())
            .or_default()
            .push(listing);
    }

    let mut bots = Vec::new();
    for plugin in installed
        .plugins
        .iter()
        .filter(|plugin| plugin.chat.is_some())
    {
        let access = |bot: &BotRef| match bot_access(&gateway, peer, &headers, bot) {
            Ok(access) => Ok(access),
            Err(error) => Err(error.to_string()),
        };
        match by_plugin.remove(&plugin.id) {
            Some(records) => {
                for record in records {
                    match access(&record.bot) {
                        Ok(access) => bots.push(SharedBot {
                            bot: record.bot,
                            user_id: Some(record.user_id),
                            title: plugin.title.clone(),
                            icon: plugin.icon.clone(),
                            access,
                        }),
                        Err(error) => {
                            return error_response(
                                StatusCode::INTERNAL_SERVER_ERROR,
                                "storage",
                                &error,
                            );
                        }
                    }
                }
            }
            None => {
                // 插件的 Bot 记录还没建出来：留一行让家人看得见“有这个 Bot、还没建好”，不伪装成没有。
                let bot = BotRef {
                    plugin_id: plugin.id.clone(),
                    bot: DEFAULT_BOT.to_owned(),
                };
                match access(&bot) {
                    Ok(access) => bots.push(SharedBot {
                        bot,
                        user_id: None,
                        title: plugin.title.clone(),
                        icon: plugin.icon.clone(),
                        access,
                    }),
                    Err(error) => {
                        return error_response(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "storage",
                            &error,
                        );
                    }
                }
            }
        }
    }
    bots.sort_by(|a, b| {
        a.title
            .cmp(&b.title)
            .then_with(|| a.bot.plugin_id.cmp(&b.bot.plugin_id))
            .then_with(|| a.bot.bot.cmp(&b.bot.bot))
    });
    Json(SharedBotsView { state, bots }).into_response()
}
