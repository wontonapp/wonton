//! 主人放宽确认的接口（ADR-0024）。只给主人（ADR-0045）：卡片都发给主人，放宽也是主人的设置。Agent 和插件都在
//! 插件网络里，两样身份都没有，打开不了。

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use hyper::{HeaderMap, Method, StatusCode};

use super::api::{acted_by, error_response, install_error, owner_denied};
use super::{Gateway, TunnelPeer};
use crate::l10n;
use crate::mcp::{Endpoint, PolicyChange};
use crate::plugin::{InstallError, PluginId};

/// 过了门的请求：改的是哪个插件的设置，谁改的。
struct Granted<'a> {
    mcp: &'a Endpoint,
    plugin: PluginId,
    by: String,
}

fn granted<'a>(
    gateway: &'a Gateway,
    peer: Option<Extension<TunnelPeer>>,
    headers: &HeaderMap,
    id: &str,
) -> Result<Granted<'a>, Box<Response>> {
    let by = acted_by(gateway, peer.as_ref(), headers);
    if let Some(denied) = owner_denied(gateway, peer, headers) {
        return Err(Box::new(denied));
    }
    let Some(mcp) = &gateway.mcp else {
        let langs = gateway.plugins.owner_languages();
        let message = l10n::text(&langs, "gateway-mcp-disabled", &[]);
        return Err(Box::new(error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "mcp_unavailable",
            &message,
        )));
    };
    let plugin = PluginId::parse(id).map_err(|e| {
        Box::new(error_response(
            StatusCode::BAD_REQUEST,
            "invalid_input",
            &e.to_string(),
        ))
    })?;
    if gateway.plugins.title(&plugin).is_none() {
        let langs = super::api::languages(headers);
        return Err(Box::new(install_error(&InstallError::not_installed(
            &langs, &plugin,
        ))));
    }
    Ok(Granted { mcp, plugin, by })
}

fn not_found(message: &str) -> Response {
    error_response(StatusCode::NOT_FOUND, "not_found", message)
}

/// 是能调工具的 Bot。
fn is_bot(gateway: &Gateway, caller: &str) -> bool {
    gateway
        .plugins
        .tool_callers()
        .iter()
        .any(|(id, _)| id.as_str() == caller)
}

/// 改一个开关：`PUT` 打开，`DELETE` 关掉。响应是改完的设置。
async fn apply(granted: Granted<'_>, change: PolicyChange<'_>, method: &Method) -> Response {
    let bus = &granted.mcp.bus;
    let enabled = method == Method::PUT;
    match bus.change_tool_policy(change, enabled, &granted.by) {
        Ok(()) => Json(bus.tool_policy_view(&granted.plugin).await).into_response(),
        Err(e) => error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "policy_unavailable",
            &e.to_string(),
        ),
    }
}

/// `GET /_wonton/api/plugins/{id}/tool-policy`
pub async fn view(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    match granted(&gateway, peer, &headers, &id) {
        Ok(granted) => {
            Json(granted.mcp.bus.tool_policy_view(&granted.plugin).await).into_response()
        }
        Err(response) => *response,
    }
}

/// `PUT`、`DELETE /_wonton/api/plugins/{id}/tool-policy/always/{caller}`
pub async fn always(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    method: Method,
    Path((id, caller)): Path<(String, String)>,
) -> Response {
    let granted = match granted(&gateway, peer, &headers, &id) {
        Ok(granted) => granted,
        Err(response) => return *response,
    };
    if !is_bot(&gateway, &caller) {
        let langs = gateway.plugins.owner_languages();
        let message = l10n::text(
            &langs,
            "gateway-bot-not-found",
            &[("caller", caller.as_str().into())],
        );
        return not_found(&message);
    }
    let change = PolicyChange::Always {
        plugin: &id,
        caller: &caller,
    };
    apply(granted, change, &method).await
}

/// `PUT`、`DELETE /_wonton/api/plugins/{id}/tool-policy/allow-all/{caller}`：`{id}` 是 Bot 所在的插件。
pub async fn allow_all(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    method: Method,
    Path((id, caller)): Path<(String, String)>,
) -> Response {
    let granted = match granted(&gateway, peer, &headers, &id) {
        Ok(granted) => granted,
        Err(response) => return *response,
    };
    if caller != id || !is_bot(&gateway, &caller) {
        let langs = gateway.plugins.owner_languages();
        let message = l10n::text(
            &langs,
            "gateway-bot-not-in-plugin",
            &[
                ("id", id.as_str().into()),
                ("caller", caller.as_str().into()),
            ],
        );
        return not_found(&message);
    }
    apply(granted, PolicyChange::AllowAll { caller: &caller }, &method).await
}

/// `PUT`、`DELETE /_wonton/api/plugins/{id}/tool-policy/read-only/{tool}`
pub async fn read_only(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    method: Method,
    Path((id, tool)): Path<(String, String)>,
) -> Response {
    let granted = match granted(&gateway, peer, &headers, &id) {
        Ok(granted) => granted,
        Err(response) => return *response,
    };
    if method == Method::PUT && granted.mcp.bus.lacks_tool(&granted.plugin, &tool).await {
        let langs = gateway.plugins.owner_languages();
        let message = l10n::text(
            &langs,
            "gateway-tool-not-found",
            &[("id", id.as_str().into()), ("tool", tool.as_str().into())],
        );
        return not_found(&message);
    }
    if method == Method::PUT && granted.mcp.bus.tool_deletes(&granted.plugin, &tool).await {
        let langs = gateway.plugins.owner_languages();
        let message = l10n::text(
            &langs,
            "gateway-tool-deletes",
            &[("tool", tool.as_str().into())],
        );
        return error_response(StatusCode::BAD_REQUEST, "invalid_input", &message);
    }
    let change = PolicyChange::ReadOnly {
        plugin: &id,
        tool: &tool,
    };
    apply(granted, change, &method).await
}

/// `PUT`、`DELETE /_wonton/api/plugins/{id}/takeover`：开始或续约、结束“我来操作”。
pub async fn takeover(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    method: Method,
    Path(id): Path<String>,
) -> Response {
    let granted = match granted(&gateway, peer, &headers, &id) {
        Ok(granted) => granted,
        Err(response) => return *response,
    };
    let bus = &granted.mcp.bus;
    if method == Method::DELETE {
        bus.end_takeover(&granted.plugin, &granted.by);
    } else if bus.serves_tools(&granted.plugin) {
        bus.renew_takeover(&granted.plugin, &granted.by);
    } else {
        let langs = gateway.plugins.owner_languages();
        let message = l10n::text(&langs, "gateway-plugin-not-running", &[]);
        return error_response(StatusCode::CONFLICT, "not_ready", &message);
    }
    StatusCode::NO_CONTENT.into_response()
}
