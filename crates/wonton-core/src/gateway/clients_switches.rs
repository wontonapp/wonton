//! 第三方客户端入口开关。

use std::sync::Arc;

use axum::Router;
use axum::extract::{Json, Path, State};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, put};
use hyper::HeaderMap;
use wonton_protocol::{
    ClientAuthKind, ClientsFailure, ClientsPlugin, EntryBlock, EntryKind, EntrySwitch,
    SetEntryRequest,
};

use super::clients_api::{failure, no_store};
use super::{Gateway, api};
use crate::clients::apply;
use crate::clients::state::Kind;
use crate::plugin::languages::Languages;
use crate::plugin::manager::ClientPlugin;

const LAN_PORT: u16 = 8483;

pub(super) fn routes() -> Router<Arc<Gateway>> {
    Router::new()
        .route(&wonton_protocol::clients_path(), get(list))
        .route(
            &format!(
                "{}/{{id}}/entries/{{entry}}",
                wonton_protocol::clients_path()
            ),
            put(set_entry),
        )
}

async fn suffix(gateway: &Gateway) -> Result<Option<String>, apply::ApplyError> {
    match &gateway.domain {
        Some(domain) => domain
            .view()
            .await
            .map(|view| view.suffix)
            .map_err(|error| {
                apply::keyed(
                    ClientsFailure::IngressUnavailable,
                    "clients-switch-domain-read",
                    vec![("error", format!("{error:#}"))],
                )
            }),
        None => Ok(None),
    }
}

async fn list(State(gateway): State<Arc<Gateway>>, headers: HeaderMap) -> Response {
    let langs = api::languages(&headers);
    let suffix = match suffix(&gateway).await {
        Ok(suffix) => suffix,
        Err(error) => return failure(error.failure, &error.localized_message(&langs)),
    };
    let clients = match gateway.plugins.client_plugins() {
        Ok(clients) => clients,
        Err(message) => {
            let error = apply::error(ClientsFailure::PluginUnavailable, message);
            return failure(error.failure, &error.localized_message(&langs));
        }
    };
    let mut views = Vec::with_capacity(clients.len());
    for client in clients {
        match client_view(&gateway, &langs, client, suffix.as_deref(), None).await {
            Ok(view) => views.push(view),
            Err(error) => return failure(error.failure, &error.localized_message(&langs)),
        }
    }
    no_store(Json(views).into_response())
}

async fn set_entry(
    State(gateway): State<Arc<Gateway>>,
    headers: HeaderMap,
    Path((id, entry)): Path<(String, String)>,
    Json(request): Json<SetEntryRequest>,
) -> Response {
    let langs = api::languages(&headers);
    let id = match crate::plugin::PluginId::parse(&id) {
        Ok(id) => id,
        Err(error) => return failure(ClientsFailure::InvalidInput, &error.to_string()),
    };
    let Some(kind) = parse_kind(&entry) else {
        return failure(
            ClientsFailure::InvalidInput,
            &crate::l10n::text(&langs, "clients-switch-kind", &[]),
        );
    };
    let blocked = match apply::set_entry(&gateway, &id, kind, request.enabled).await {
        Ok(blocked) => blocked,
        Err(error) => return failure(error.failure, &error.localized_message(&langs)),
    };
    let client = match gateway.plugins.client_plugins() {
        Ok(clients) => clients.into_iter().find(|client| client.id == id),
        Err(message) => {
            let error = apply::error(ClientsFailure::PluginUnavailable, message);
            return failure(error.failure, &error.localized_message(&langs));
        }
    };
    let Some(client) = client else {
        return failure(
            ClientsFailure::PluginUnavailable,
            &crate::l10n::text(&langs, "clients-switch-uninstalled", &[]),
        );
    };
    let suffix = match suffix(&gateway).await {
        Ok(suffix) => suffix,
        Err(error) => return failure(error.failure, &error.localized_message(&langs)),
    };
    match client_view(&gateway, &langs, client, suffix.as_deref(), blocked).await {
        Ok(view) => no_store(Json(view).into_response()),
        Err(error) => failure(error.failure, &error.localized_message(&langs)),
    }
}

fn parse_kind(entry: &str) -> Option<Kind> {
    match entry {
        "lan" => Some(Kind::Lan),
        "public" => Some(Kind::Public),
        _ => None,
    }
}

async fn client_view(
    gateway: &Gateway,
    langs: &Languages,
    mut client: ClientPlugin,
    suffix: Option<&str>,
    blocked_override: Option<EntryBlock>,
) -> Result<ClientsPlugin, apply::ApplyError> {
    let blocked = match blocked_override {
        Some(blocked) => Some(blocked),
        None => apply::block_reason(gateway, &client).await?,
    };
    // setup 检查可能已持久关闭入口；地址与开关必须来自检查完成后的同一份记录。
    client.record = gateway
        .plugins
        .store()
        .load(&client.id)
        .map_err(|error| {
            apply::keyed(
                ClientsFailure::PluginUnavailable,
                "clients-switch-record-read",
                vec![
                    ("plugin", client.id.to_string()),
                    ("error", error.to_string()),
                ],
            )
        })?
        .ok_or_else(|| {
            apply::error(
                ClientsFailure::PluginUnavailable,
                "clients-switch-uninstalled",
            )
        })?;
    let label = client.record.entry_label.as_deref().ok_or_else(|| {
        apply::keyed(
            ClientsFailure::PluginUnavailable,
            "clients-switch-no-label",
            vec![("plugin", client.id.to_string())],
        )
    })?;
    let lan_url = suffix.and_then(|suffix| {
        client
            .record
            .clients_entries
            .lan
            .then(|| format!("https://{label}.lan.{suffix}:{LAN_PORT}"))
    });
    let public_url = suffix.and_then(|suffix| {
        (client.record.clients_entries.public && gateway.public_access_enabled())
            .then(|| format!("https://{label}.{suffix}"))
    });
    Ok(ClientsPlugin {
        id: client.id.to_string(),
        label: langs.pick_owned(&client.title),
        lan_url,
        public_url,
        entries: entry_switches(&client),
        auth: client.auth,
        username: match client.auth {
            ClientAuthKind::Gateway => Some(gateway.owner.username().to_owned()),
            _ => client.username.as_deref().map(|username| {
                username
                    .replace("{user}", gateway.owner.username())
                    .replace(
                        "{email}",
                        &crate::owner::internal_email(gateway.owner.username()),
                    )
            }),
        },
        passwords: password_summaries(gateway, &client, langs)?,
        public_rules: client.public_rules.clone(),
        blocked,
    })
}

fn password_summaries(
    gateway: &Gateway,
    client: &ClientPlugin,
    langs: &Languages,
) -> Result<Vec<wonton_protocol::PasswordSummary>, apply::ApplyError> {
    if client.auth == ClientAuthKind::App {
        return Ok(Vec::new());
    }
    let unavailable = || {
        apply::error(
            ClientsFailure::PluginUnavailable,
            crate::l10n::text(langs, "clients-passwords-unavailable", &[]),
        )
    };
    let store = gateway
        .plugins
        .client_passwords
        .as_ref()
        .ok_or_else(unavailable)?;
    store
        .summaries(client.id.as_str(), |device| {
            if device == crate::clients::passwords::OWNER_TOKEN {
                return Some(gateway.owner.username().to_owned());
            }
            Some(
                device
                    .parse::<iroh::EndpointId>()
                    .ok()
                    .and_then(|id| gateway.tunnel.devices.get(&id))
                    .and_then(|device| device.name)
                    .unwrap_or_else(|| device.to_owned()),
            )
        })
        .map_err(|error| {
            tracing::error!(%error, "专用密码库读取失败");
            unavailable()
        })
}

/// TCP 入口的转发（ADR-0021“普通 TCP 转发”）还没接，先只列 HTTP 的两项。
fn entry_switches(client: &ClientPlugin) -> Vec<EntrySwitch> {
    if !client.http {
        return Vec::new();
    }
    vec![
        EntrySwitch {
            kind: EntryKind::Lan,
            enabled: client.record.clients_entries.lan,
        },
        EntrySwitch {
            kind: EntryKind::Public,
            enabled: client.record.clients_entries.public,
        },
    ]
}

#[cfg(test)]
#[path = "clients_api_tests.rs"]
mod clients_api_tests;
