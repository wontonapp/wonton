//! 访客链接的管理接口（ADR-0090 第 5 节）：挂在 [`super::clients_api`] 的中间件后面，只给主人。
//!
//! 只有声明了 `webui.guests` 的插件能建。建、收回和每次取地址都写审计；列表不记。

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::{Extension, Path, State};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use hyper::{HeaderMap, StatusCode};
use serde::Serialize;
use wonton_protocol::{ClientsFailure, GuestLink, NewGuestLinkRequest};

use super::Gateway;
use super::TunnelPeer;
use super::api::{acted_by, languages};
use super::clients_api::{failure, failure_with_status, no_store};
use crate::clients::external_url::lan_port;
use crate::clients::guests::{Guest, GuestError};
use crate::l10n;
use crate::plugin::PluginId;
use crate::plugin::languages::Languages;
use crate::plugin::manager::Route;
use crate::plugin::store::InstalledRecord;

pub fn routes() -> Router<Arc<Gateway>> {
    let base = wonton_protocol::clients_path();
    Router::new()
        .route(&format!("{base}/{{id}}/guests"), get(list).post(create))
        .route(
            &format!("{base}/{{id}}/guests/{{guest}}"),
            get(read).delete(revoke),
        )
}

/// 一条需要记下来的访客链接操作。
#[derive(Serialize)]
struct GuestEvent<'a> {
    event: &'static str,
    /// `device:<设备 ID>` 或 `owner_token`，口径同 [`acted_by`]。
    by: &'a str,
    plugin: &'a str,
    guest: &'a str,
    label: &'a str,
    /// 入口容器写的真实来源；解析不出就不写。
    source: Option<&'a str>,
}

async fn list(
    State(gateway): State<Arc<Gateway>>,
    headers: HeaderMap,
    Path(plugin): Path<String>,
) -> Response {
    let langs = languages(&headers);
    let Some(store) = &gateway.plugins.client_guests else {
        return unavailable(&langs);
    };
    let (id, _route) = match declared(&gateway, &plugin, &langs).await {
        Ok(found) => found,
        Err(response) => return response,
    };
    let links: Vec<GuestLink> = store
        .list(id.as_str())
        .into_iter()
        .map(|guest| view(guest, None))
        .collect();
    no_store(Json(links).into_response())
}

async fn create(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path(plugin): Path<String>,
    Json(request): Json<NewGuestLinkRequest>,
) -> Response {
    let langs = languages(&headers);
    let Some(store) = &gateway.plugins.client_guests else {
        return unavailable(&langs);
    };
    let (id, route) = match declared(&gateway, &plugin, &langs).await {
        Ok(found) => found,
        Err(response) => return response,
    };
    if !route.web_serving {
        return failure(
            ClientsFailure::PluginUnavailable,
            &l10n::text(&langs, "clients-plugin-unavailable", &[]),
        );
    }
    let record = match gateway.plugins.store().load(&id) {
        Ok(Some(record)) => record,
        Ok(None) => {
            return failure(
                ClientsFailure::PluginUnavailable,
                &l10n::text(&langs, "clients-plugin-unavailable", &[]),
            );
        }
        Err(error) => {
            tracing::error!(plugin = %id, %error, "读访客链接的插件记录失败");
            return failure(
                ClientsFailure::PluginUnavailable,
                &l10n::text(&langs, "clients-guests-unavailable", &[]),
            );
        }
    };
    let Some(base) = reachable_base(&gateway, &record) else {
        return failure(
            ClientsFailure::EntryClosed,
            &l10n::text(&langs, "clients-guests-entry-closed", &[]),
        );
    };
    let created = match store.create(
        id.as_str(),
        &request.label,
        request.ttl_seconds,
        request.path.as_deref(),
    ) {
        Ok(created) => created,
        Err(error) => return guest_error(error, &langs),
    };
    audit(
        &gateway,
        GuestEvent {
            event: "guest_link_create",
            by: &acted_by(&gateway, peer.as_ref(), &headers),
            plugin: id.as_str(),
            guest: &created.id,
            label: &created.label,
            source: source(&headers),
        },
    );
    let Some(url) = url_for(&gateway, &base, id.as_str(), &created) else {
        return guest_error(GuestError::NotFound, &langs);
    };
    no_store((StatusCode::CREATED, Json(view(created, Some(url)))).into_response())
}

async fn read(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path((plugin, guest)): Path<(String, String)>,
) -> Response {
    let langs = languages(&headers);
    let Some(store) = &gateway.plugins.client_guests else {
        return unavailable(&langs);
    };
    let (id, _route) = match declared(&gateway, &plugin, &langs).await {
        Ok(found) => found,
        Err(response) => return response,
    };
    let link = match store.get(id.as_str(), &guest) {
        Ok(link) => link,
        Err(error) => return guest_error(error, &langs),
    };
    let record = match gateway.plugins.store().load(&id) {
        Ok(Some(record)) => record,
        _ => {
            return failure(
                ClientsFailure::PluginUnavailable,
                &l10n::text(&langs, "clients-plugin-unavailable", &[]),
            );
        }
    };
    let Some(base) = reachable_base(&gateway, &record) else {
        return failure(
            ClientsFailure::EntryClosed,
            &l10n::text(&langs, "clients-guests-entry-closed", &[]),
        );
    };
    audit(
        &gateway,
        GuestEvent {
            event: "guest_link_value",
            by: &acted_by(&gateway, peer.as_ref(), &headers),
            plugin: id.as_str(),
            guest: &link.id,
            label: &link.label,
            source: source(&headers),
        },
    );
    let Some(url) = url_for(&gateway, &base, id.as_str(), &link) else {
        return guest_error(GuestError::NotFound, &langs);
    };
    no_store(Json(view(link, Some(url))).into_response())
}

async fn revoke(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path((plugin, guest)): Path<(String, String)>,
) -> Response {
    let langs = languages(&headers);
    let Some(store) = &gateway.plugins.client_guests else {
        return unavailable(&langs);
    };
    let (id, _route) = match declared(&gateway, &plugin, &langs).await {
        Ok(found) => found,
        Err(response) => return response,
    };
    let link = match store.get(id.as_str(), &guest) {
        Ok(link) => link,
        Err(error) => return guest_error(error, &langs),
    };
    match store.revoke(id.as_str(), &guest) {
        Ok(()) => {}
        Err(error) => return guest_error(error, &langs),
    }
    audit(
        &gateway,
        GuestEvent {
            event: "guest_link_revoke",
            by: &acted_by(&gateway, peer.as_ref(), &headers),
            plugin: id.as_str(),
            guest: &link.id,
            label: &link.label,
            source: source(&headers),
        },
    );
    no_store(StatusCode::NO_CONTENT.into_response())
}

/// 插件存在、装着并且声明了 `webui.guests`；否则回 404（没装或没声明）。
#[allow(clippy::result_large_err)] // 错误就是要回给 HTTP 层的响应。
async fn declared(
    gateway: &Gateway,
    plugin: &str,
    langs: &Languages,
) -> Result<(PluginId, Route), Response> {
    let Ok(id) = PluginId::parse(plugin) else {
        return Err(not_declared(langs));
    };
    // 没装和装了没声明对调用方一样：这个能力不在。
    if !gateway.plugins.is_installed(&id) {
        return Err(not_declared(langs));
    }
    let Some(route) = gateway.plugins.route(&id.slug(), "/") else {
        return Err(not_declared(langs));
    };
    if !route.guests {
        return Err(not_declared(langs));
    }
    Ok((id, route))
}

/// 没声明 `webui.guests` 的插件不是没有这个东西，所以用 404 而不是 `Other` 的默认 500。
fn not_declared(langs: &Languages) -> Response {
    failure_with_status(
        StatusCode::NOT_FOUND,
        ClientsFailure::Other,
        &l10n::text(langs, "clients-guests-not-declared", &[]),
    )
}

/// 现在能用的那个入口的地址前半段：公网入口开着（总开关也开着）用公网，否则用局域网；都没开是 `None`。
fn reachable_base(gateway: &Gateway, record: &InstalledRecord) -> Option<String> {
    let suffix = gateway.domain.as_ref()?.suffix().ok().flatten()?;
    let label = record.entry_label.as_deref()?;
    if record.clients_entries.public && gateway.public_access_enabled() {
        return Some(format!("https://{label}.{suffix}"));
    }
    record
        .clients_entries
        .lan
        .then(|| format!("https://{label}.lan.{suffix}:{}", lan_port()))
}

/// 一条链接的令牌；列表和取地址都要拼进 URL，库里只在需要时给出。
/// 拼完整地址；令牌取不到（并发收回）时由调用方回“链接已经不在”。
fn url_for(gateway: &Gateway, base: &str, plugin: &str, guest: &Guest) -> Option<String> {
    let token = gateway
        .plugins
        .client_guests
        .as_ref()?
        .token(plugin, &guest.id)?;
    Some(format!("{base}/_wonton/guest/{token}"))
}

fn view(guest: Guest, url: Option<String>) -> GuestLink {
    GuestLink {
        id: guest.id,
        label: guest.label,
        expires_at_ms: guest.expires_at_ms,
        path: guest.path,
        last_used_at_ms: guest.last_used_at_ms,
        url,
    }
}

fn guest_error(error: GuestError, langs: &Languages) -> Response {
    let (failure, key, status) = match error {
        GuestError::NotFound => (
            ClientsFailure::EntryClosed,
            "clients-guests-not-found",
            StatusCode::CONFLICT,
        ),
        GuestError::TooMany => (
            ClientsFailure::Conflict,
            "clients-guests-max",
            StatusCode::CONFLICT,
        ),
        GuestError::InvalidTtl => (
            ClientsFailure::InvalidInput,
            "clients-guests-ttl",
            StatusCode::BAD_REQUEST,
        ),
        GuestError::InvalidPath => (
            ClientsFailure::InvalidInput,
            "clients-guests-path",
            StatusCode::BAD_REQUEST,
        ),
        GuestError::InvalidLabel => (
            ClientsFailure::InvalidInput,
            "clients-guests-label",
            StatusCode::BAD_REQUEST,
        ),
        other => {
            tracing::error!(%other, "访客链接库读写失败");
            return failure(
                ClientsFailure::PluginUnavailable,
                &l10n::text(langs, "clients-guests-unavailable", &[]),
            );
        }
    };
    failure_with_status(status, failure, &l10n::text(langs, key, &[]))
}

fn unavailable(langs: &Languages) -> Response {
    failure(
        ClientsFailure::PluginUnavailable,
        &l10n::text(langs, "clients-guests-unavailable", &[]),
    )
}

fn source(headers: &HeaderMap) -> Option<&str> {
    headers
        .get("x-real-ip")
        .and_then(|value| value.to_str().ok())
}

fn audit(gateway: &Gateway, event: GuestEvent<'_>) {
    if let Some(audit) = &gateway.audit {
        audit.record(&event);
    }
}
