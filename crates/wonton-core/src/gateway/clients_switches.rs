//! 第三方客户端入口开关。

use std::sync::Arc;

use axum::Router;
use axum::extract::{Json, Path, State};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, put};
use hyper::HeaderMap;
use wonton_protocol::{
    ClientsFailure, ClientsPlugin, EntryBlock, EntryKind, EntrySwitch, SetEntryRequest,
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

async fn suffix(gateway: &Gateway) -> Result<Option<String>, Response> {
    match &gateway.domain {
        Some(domain) => domain
            .view()
            .await
            .map(|view| view.suffix)
            .map_err(|error| failure(ClientsFailure::IngressUnavailable, &format!("{error:#}"))),
        None => Ok(None),
    }
}

async fn list(State(gateway): State<Arc<Gateway>>, headers: HeaderMap) -> Response {
    let langs = api::languages(&headers);
    let suffix = match suffix(&gateway).await {
        Ok(suffix) => suffix,
        Err(response) => return response,
    };
    let clients = match gateway.plugins.client_plugins() {
        Ok(clients) => clients,
        Err(error) => return failure(ClientsFailure::PluginUnavailable, &error),
    };
    let mut views = Vec::with_capacity(clients.len());
    for client in clients {
        match client_view(&gateway, &langs, &client, suffix.as_deref(), None).await {
            Ok(view) => views.push(view),
            Err(response) => return response,
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
        Err(error) => return failure(ClientsFailure::PluginUnavailable, &error),
    };
    let Some(client) = client else {
        return failure(
            ClientsFailure::PluginUnavailable,
            &crate::l10n::text(&langs, "clients-switch-uninstalled", &[]),
        );
    };
    let suffix = match suffix(&gateway).await {
        Ok(suffix) => suffix,
        Err(response) => return response,
    };
    match client_view(&gateway, &langs, &client, suffix.as_deref(), blocked).await {
        Ok(view) => no_store(Json(view).into_response()),
        Err(response) => response,
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
    client: &ClientPlugin,
    suffix: Option<&str>,
    blocked_override: Option<EntryBlock>,
) -> Result<ClientsPlugin, Response> {
    let label = client.record.entry_label.as_deref().ok_or_else(|| {
        failure(
            ClientsFailure::PluginUnavailable,
            &format!("客户端插件 {} 没有入口标签", client.id),
        )
    })?;
    let blocked = match blocked_override {
        Some(blocked) => Some(blocked),
        None => apply::block_reason(gateway, client)
            .await
            .map_err(|error| failure(error.failure, &error.localized_message(langs)))?,
    };
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
        entries: entry_switches(client),
        auth: client.auth,
        username: client
            .username
            .as_deref()
            .map(|username| username.replace("{user}", gateway.owner.username())),
        passwords: Vec::new(),
        public_rules: client.public_rules.clone(),
        blocked,
    })
}

fn entry_switches(client: &ClientPlugin) -> Vec<EntrySwitch> {
    let mut entries = Vec::new();
    if client.http {
        entries.extend([
            EntrySwitch {
                kind: EntryKind::Lan,
                enabled: client.record.clients_entries.lan,
            },
            EntrySwitch {
                kind: EntryKind::Public,
                enabled: client.record.clients_entries.public,
            },
        ]);
    }
    entries.extend(client.tcp_names.iter().cloned().map(|name| EntrySwitch {
        kind: EntryKind::Tcp { name },
        enabled: false,
    }));
    entries
}

#[cfg(test)]
#[path = "clients_api_tests.rs"]
mod clients_api_tests;
