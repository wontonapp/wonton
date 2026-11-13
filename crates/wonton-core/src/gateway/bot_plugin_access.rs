//! 主人对单个 Bot 开关一个插件的工具（ADR-0066 第 5 节）。只给主人：和插件范围一样是 `owner_only`。总线据此过滤
//! 这个 Bot 的 `tools/list`、拦它的 `tools/call`（`crate::mcp` 的 `bot_access`）。

use wonton_protocol::PermissionItem;

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use hyper::{HeaderMap, Method, StatusCode};
use wonton_protocol::{AccessSource, BotPluginAccess, BotPluginAccessUpdate};

use super::api::{error_response, install_error, languages};
use super::api_body::invalid_body;
use super::{Gateway, TunnelPeer, permissions};
use crate::l10n;
use crate::plugin::bots::check_bot_name;
use crate::plugin::tool_policy::PolicyUnavailable;
use crate::plugin::{InstallError, PluginId};

/// 认过的路径：`plugin` 的 Bot `bot` 对 `target`。
struct Target {
    plugin: PluginId,
    bot: String,
    target: PluginId,
}

fn target(
    gateway: &Gateway,
    headers: &HeaderMap,
    (id, bot, target): (String, String, String),
) -> Result<Target, Box<Response>> {
    let invalid = |e: String| Box::new(install_error(&invalid_body(headers, e)));
    let parse = |id: &str| PluginId::parse(id).map_err(|e| invalid(e.to_string()));
    let plugin = parse(&id)?;
    let target = parse(&target)?;
    check_bot_name(&bot).map_err(invalid)?;
    let langs = languages(headers);
    match gateway.plugins.bot_localpart(&plugin, &bot) {
        Ok(Some(_)) => {}
        Ok(None) => {
            let message = l10n::text(
                &langs,
                "gateway-bot-not-in-plugin",
                &[("id", id.as_str().into()), ("caller", bot.as_str().into())],
            );
            return Err(Box::new(error_response(
                StatusCode::NOT_FOUND,
                "not_found",
                &message,
            )));
        }
        Err(e) => return Err(Box::new(install_error(&InstallError::from(e)))),
    }
    if gateway.plugins.title(&target).is_none() {
        return Err(Box::new(install_error(&InstallError::not_installed(
            &langs, &target,
        ))));
    }
    Ok(Target {
        plugin,
        bot,
        target,
    })
}

fn unavailable(e: &PolicyUnavailable) -> Response {
    error_response(
        StatusCode::SERVICE_UNAVAILABLE,
        "policy_unavailable",
        &e.to_string(),
    )
}

/// 现在的设置。范围只有“全局”在生效，所以来源都是全局。
fn current(gateway: &Gateway, t: &Target) -> Response {
    let policy = gateway.plugins.tool_policy();
    match policy.is_bot_off(t.plugin.as_str(), &t.bot, t.target.as_str()) {
        Ok(off) => Json(BotPluginAccess {
            plugin_id: t.target.to_string(),
            source: AccessSource::Global,
            enabled: !off,
        })
        .into_response(),
        Err(e) => unavailable(&e),
    }
}

/// `GET`、`PUT /_wonton/api/bots/{id}/{bot}/plugin-access/{target}`：`PUT` 的请求体是 [`BotPluginAccessUpdate`]；
/// 两个都返回改完的 [`BotPluginAccess`]。
pub(super) async fn bot_access(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    method: Method,
    Path(path): Path<(String, String, String)>,
    body: Bytes,
) -> Response {
    if let Some(denied) =
        permissions::require(&gateway, peer, &headers, PermissionItem::InstallApps)
    {
        return denied;
    }
    let update = if method == Method::PUT {
        match serde_json::from_slice::<BotPluginAccessUpdate>(&body) {
            Ok(update) => Some(update),
            Err(error) => return install_error(&invalid_body(&headers, error)),
        }
    } else {
        None
    };
    let t = match target(&gateway, &headers, path) {
        Ok(t) => t,
        Err(response) => return *response,
    };
    if let Some(update) = update {
        let saved =
            gateway
                .plugins
                .set_bot_plugin_access(&t.plugin, &t.bot, &t.target, update.enabled);
        if let Err(e) = saved {
            return unavailable(&e);
        }
    }
    current(&gateway, &t)
}

#[cfg(test)]
#[path = "bot_plugin_access_tests.rs"]
mod tests;
