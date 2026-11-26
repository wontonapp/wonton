use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use hyper::HeaderMap;
use serde::Deserialize;
use wonton_protocol::{AddedTools, PermissionItem, UpgradeRequest, UpgradeResult};

use super::api_switch::{settings, upload_parts};
use super::{
    Gateway, InstallQuery, TunnelPeer, comma_list, install_error, install_gate_checks,
    install_gates, json_body, languages, package, permissions, requester, response_for_requester,
};
use crate::l10n;
use crate::plugin::{InstallError, PluginId, UpgradeTo};

/// 升级预览的 JSON 请求体：`{}` 是升到提示的版本，带 `source` 是换到这个来源。
#[derive(Deserialize)]
struct UpgradeFrom {
    #[serde(default)]
    source: Option<String>,
}

pub async fn preview_upgrade(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: Bytes,
) -> Response {
    if let Some(denied) =
        permissions::require(&gateway, peer, &headers, PermissionItem::InstallApps)
    {
        return denied;
    }
    let id = match PluginId::parse(&id) {
        Ok(id) => id,
        Err(e) => return install_error(&InstallError::Invalid(e.to_string())),
    };
    let to = match json_body::<UpgradeFrom>(&headers, &body) {
        Some(Err(e)) => return install_error(&e),
        Some(Ok(UpgradeFrom {
            source: Some(source),
        })) => UpgradeTo::Source(source),
        Some(Ok(UpgradeFrom { source: None })) => UpgradeTo::Offered,
        None if body.is_empty() => UpgradeTo::Offered,
        None => match package(&languages(&headers), &headers, &body) {
            Ok(package) => UpgradeTo::Upload(Box::new(package)),
            Err(e) => return install_error(&e),
        },
    };
    let langs = languages(&headers);
    match gateway.plugins.preview_upgrade(&id, to, None, &langs).await {
        Ok(mut preview) => {
            if let Ok(person) = requester(&gateway, peer, &headers) {
                install_gates::project_preview(&gateway, &person, &mut preview);
            }
            Json(preview).into_response()
        }
        Err(e) => install_error(&e),
    }
}

/// 卡片批准后真正执行升级：来源或上传，回升级结果（ADR-0088 的门槛在入口，不在这里）。
async fn perform_upgrade(
    gateway: Arc<Gateway>,
    id: PluginId,
    headers: HeaderMap,
    query: InstallQuery,
    body: Bytes,
) -> Result<UpgradeResult, InstallError> {
    let langs = languages(&headers);
    let before = match &gateway.mcp {
        Some(mcp) => mcp.bus.tool_names(&id).await,
        None => Err(l10n::text(&langs, "gateway-mcp-disabled", &[])),
    };
    let upgraded = match json_body::<UpgradeRequest>(&headers, &body) {
        Some(Err(e)) => return Err(e),
        Some(Ok(request)) => gateway.plugins.upgrade_source(&id, &request, &langs).await,
        None => match upload_parts(&languages(&headers), &headers, &body, &query)
            .and_then(|parts| Ok((parts, settings(&headers)?)))
        {
            Ok(((grant, enabled, secrets, package), given)) => {
                let plugins = &gateway.plugins;
                let filled = plugins.store().secrets().load(&id).ok().flatten();
                plugins.store_input_values(&package.manifest, &secrets, filled.as_ref(), &langs)?;
                plugins
                    .upgrade_upload(
                        &id,
                        package,
                        grant,
                        enabled,
                        query.lan_webui,
                        &comma_list(&query.devices),
                        &given,
                        &langs,
                    )
                    .await
            }
            Err(e) => return Err(e),
        },
    };
    upgraded?;
    // 主人经文件或来源升级后，插件不再归 Agent（ADR-0033）：归属和“每次都问”一起清掉。
    if let Err(e) = gateway.plugins.set_writer(&id, None) {
        tracing::error!(plugin = %id, error = %e, "清插件的归属失败");
    }
    let added_tools = match &gateway.mcp {
        Some(mcp) => mcp.bus.added_tools(&id, before).await,
        None => AddedTools::NotCompared {
            reason: l10n::text(&langs, "gateway-mcp-disabled", &[]),
        },
    };
    let plugin = gateway
        .plugins
        .installed_plugin(&id, &langs)
        .ok_or_else(|| InstallError::NotInstalled(id.to_string()))?;
    Ok(UpgradeResult {
        plugin,
        added_tools,
    })
}

pub async fn upgrade(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(query): Query<InstallQuery>,
    body: Bytes,
) -> Response {
    let id = match PluginId::parse(&id) {
        Ok(id) => id,
        Err(e) => return install_error(&InstallError::Invalid(e.to_string())),
    };
    let langs = languages(&headers);
    let person = match requester(&gateway, peer, &headers) {
        Ok(person) => person,
        Err(response) => return *response,
    };
    // 不能升级的人在读请求体之前就被挡下（ADR-0088 的权限项按人判断，与请求内容无关）。
    if let Some(denied) = install_gates::deny_check(&gateway, &person) {
        return denied;
    }
    let finish = {
        let headers = headers.clone();
        let query = query.clone();
        let body = body.clone();
        let id = id.clone();
        let person = person.clone();
        move |gateway: Arc<Gateway>| {
            let upgraded = perform_upgrade(gateway.clone(), id, headers.clone(), query, body);
            async move {
                let result = upgraded.await;
                response_for_requester(
                    &gateway,
                    peer,
                    &headers,
                    &person,
                    result,
                    |gateway, peer, person, mut value| match super::bot_projection::installed(
                        gateway,
                        peer,
                        &headers,
                        person,
                        &mut value.plugin,
                    ) {
                        Ok(()) => Json(value).into_response(),
                        Err(response) => *response,
                    },
                )
            }
        }
    };
    // 门槛要升级预览的内容写卡片，先把请求体读出来；执行用的是同一份请求体。
    let ask = match install_gate_checks::UpgradeAsk::read(&headers, &body, &query) {
        Ok(ask) => ask,
        Err(e) => return install_error(&e),
    };
    let gate = install_gates::upgrade(&gateway, peer, &headers, &person, &id, ask, finish).await;
    match install_gates::settle(gate, "upgrade", &langs).await {
        Ok(response) | Err(response) => response,
    }
}
