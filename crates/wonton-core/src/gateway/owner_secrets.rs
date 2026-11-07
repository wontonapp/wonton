//! 主人在 App 里查看插件密钥（#339）。只给主人（[`api::owner_denied`]，ADR-0045 的“谁能做什么”）：
//! 主人令牌或主人名下的设备；家人和 Bot 的设备都回 403 `owner_only`，插件和匿名请求没有主人令牌，
//! 按未登录拒。插件的网页带着设备身份，但不是主人的，一样进不来——它不能拿设备身份去读主人的密钥。
//!
//! | 方法 | 路径 | 说明 |
//! | --- | --- | --- |
//! | GET | `/_wonton/api/plugins/{id}/secrets` | 清单里声明了 `show_owner` 的密钥：名字和 label，不带值（`OwnerPluginSecrets`） |
//! | GET | `/_wonton/api/plugins/{id}/secrets/{name}` | 一项密钥的值（`OwnerPluginSecretValue`）；每次取值写一条审计（谁、哪个插件、哪个密钥，不写值） |
//!
//! 密钥的值和恢复密钥同类（[`SecretString`]，`Debug` 打码）：响应带 `Cache-Control: no-store`，
//! App 点开时才取、拿到即显示，不缓存。

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use hyper::header::{CACHE_CONTROL, HeaderValue};
use hyper::{HeaderMap, StatusCode};
use serde::Serialize;

use super::api::{acted_by, error_response, languages, owner_denied};
use super::{Gateway, TunnelPeer};
use crate::l10n;
use crate::plugin::languages::Languages;
use crate::plugin::{PluginId, SecretValue, ViewableSecrets};
use wonton_protocol::OwnerPluginSecrets;

/// 取到值才写的一条审计：谁看了哪个插件的哪个密钥。值不进审计。
#[derive(Serialize)]
struct SecretViewEvent<'a> {
    event: &'static str,
    /// `device:<设备 ID>` 或 `owner_token`，口径同 [`acted_by`]。
    by: &'a str,
    plugin: &'a str,
    secret: &'a str,
}

/// 密钥的值不能留在任何缓存里：这个接口面的成功和错误响应都不缓存。
fn no_store(response: Response) -> Response {
    let (mut parts, body) = response.into_parts();
    parts
        .headers
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Response::from_parts(parts, body)
}

fn not_installed(langs: &Languages, id: &PluginId) -> Response {
    no_store(error_response(
        StatusCode::NOT_FOUND,
        "not_installed",
        &l10n::text(
            langs,
            "gateway-owner-secrets-plugin-not-installed",
            &[("id", id.as_str().into())],
        ),
    ))
}

fn unreadable(langs: &Languages, id: &PluginId) -> Response {
    no_store(error_response(
        StatusCode::SERVICE_UNAVAILABLE,
        "not_ready",
        &l10n::text(
            langs,
            "gateway-owner-secrets-record-unreadable",
            &[("id", id.as_str().into())],
        ),
    ))
}

/// `GET /_wonton/api/plugins/{id}/secrets`
pub async fn list(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Some(denied) = owner_denied(&gateway, peer, &headers) {
        return denied;
    }
    let id = match PluginId::parse(&id) {
        Ok(id) => id,
        Err(e) => {
            return no_store(error_response(
                StatusCode::BAD_REQUEST,
                "invalid_input",
                &e.to_string(),
            ));
        }
    };
    let langs = languages(&headers);
    let secrets = match gateway.plugins.owner_viewable_secrets(&id, &langs) {
        Some(ViewableSecrets::Listed(secrets)) => secrets,
        Some(ViewableSecrets::Unreadable) => return unreadable(&langs, &id),
        None => return not_installed(&langs, &id),
    };
    no_store(Json(OwnerPluginSecrets { secrets }).into_response())
}

/// `GET /_wonton/api/plugins/{id}/secrets/{name}`
pub async fn value(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path((id, name)): Path<(String, String)>,
) -> Response {
    if let Some(denied) = owner_denied(&gateway, peer, &headers) {
        return denied;
    }
    let id = match PluginId::parse(&id) {
        Ok(id) => id,
        Err(e) => {
            return no_store(error_response(
                StatusCode::BAD_REQUEST,
                "invalid_input",
                &e.to_string(),
            ));
        }
    };
    let langs = languages(&headers);
    let value = match gateway.plugins.owner_secret_value(&id, &name) {
        Some(SecretValue::Value(value)) => value,
        Some(SecretValue::Unreadable) => return unreadable(&langs, &id),
        Some(SecretValue::NotViewable) => {
            return no_store(error_response(
                StatusCode::NOT_FOUND,
                "not_found",
                &l10n::text(
                    &langs,
                    "gateway-owner-secrets-not-viewable",
                    &[("id", id.as_str().into()), ("name", name.as_str().into())],
                ),
            ));
        }
        Some(SecretValue::NotGenerated) => {
            return no_store(error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "not_ready",
                &l10n::text(
                    &langs,
                    "gateway-owner-secrets-not-generated",
                    &[("name", name.as_str().into())],
                ),
            ));
        }
        Some(SecretValue::Store(e)) => {
            return no_store(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage",
                &l10n::text(
                    &langs,
                    "gateway-owner-secrets-store-failed",
                    &[("error", e.to_string().into())],
                ),
            ));
        }
        Some(SecretValue::NotText) => {
            return no_store(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage",
                &l10n::text(
                    &langs,
                    "gateway-owner-secrets-not-text",
                    &[("name", name.as_str().into())],
                ),
            ));
        }
        None => return not_installed(&langs, &id),
    };
    // 先记账再给值；审计没接线时不给值（生产路径 main.rs 总是接上）。
    let Some(audit) = &gateway.audit else {
        return no_store(error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "not_ready",
            &l10n::text(&langs, "gateway-owner-secrets-audit-unavailable", &[]),
        ));
    };
    audit.record(&SecretViewEvent {
        event: "plugin_secret_view",
        by: &acted_by(&gateway, peer.as_ref(), &headers),
        plugin: id.as_str(),
        secret: &name,
    });
    no_store(Json(value).into_response())
}
