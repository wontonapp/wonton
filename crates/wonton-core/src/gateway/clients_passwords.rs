//! 专用密码的管理接口（ADR-0021「在 App 里」）：挂在 [`super::clients_api`] 的中间件后面，
//! 只有主人（令牌或已配对设备）能调。值每次取都记审计、具体到设备；吊销下一个请求生效。

use std::sync::Arc;

use axum::Extension;
use axum::Json;
use axum::Router;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use hyper::{HeaderMap, StatusCode};
use wonton_protocol::{NewPasswordRequest, PasswordValue, SecretString};

use super::Gateway;
use super::TunnelPeer;
use super::api::{acted_by, languages};
use super::clients_api::{failure, no_store, unavailable};
use crate::clients::passwords::PasswordError;
use crate::l10n;
use crate::plugin::languages::Languages;
use wonton_protocol::ClientsFailure;

pub fn routes() -> Router<Arc<Gateway>> {
    Router::new()
        .route(
            &wonton_protocol::clients_passwords_path("{id}"),
            post(create),
        )
        .route(
            &wonton_protocol::clients_password_path("{id}", "{pw}"),
            get(read_value).delete(revoke),
        )
}

/// `POST`：给插件新建一个专用密码，回 `201` 和值。
async fn create(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path(plugin): Path<String>,
    Json(request): Json<NewPasswordRequest>,
) -> Response {
    let langs = languages(&headers);
    let Some(store) = &gateway.plugins.client_passwords else {
        return unavailable(&langs);
    };
    // 设备身份：经隧道的已配对设备记 ID（吊销按它连带），名字给界面显示。
    let device = peer.as_ref().map(|Extension(peer)| {
        let name = gateway
            .tunnel
            .devices
            .get(&peer.0)
            .and_then(|device| device.name);
        (peer.0.to_string(), name)
    });
    match store.create(
        &plugin,
        &request.label,
        device
            .as_ref()
            .map(|(id, name)| (id.as_str(), name.as_deref())),
    ) {
        Ok(created) => no_store(
            (
                StatusCode::CREATED,
                Json(PasswordValue {
                    username: gateway.plugins.owner_username(),
                    password: SecretString::new(created.value),
                }),
            )
                .into_response(),
        ),
        Err(PasswordError::InvalidLabel) => failure(
            ClientsFailure::InvalidInput,
            &l10n::text(&langs, "clients-password-label-invalid", &[]),
        ),
        Err(error) => storage_failure(&langs, error),
    }
}

/// `GET`：取值；每次都记审计、具体到设备（ADR-0021「在 App 里」）。
async fn read_value(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path((plugin, id)): Path<(String, String)>,
) -> Response {
    let langs = languages(&headers);
    let Some(store) = &gateway.plugins.client_passwords else {
        return unavailable(&langs);
    };
    let device = peer.as_ref().map(|Extension(peer)| peer.0.to_string());
    match store.read(&plugin, &id, device.as_deref()) {
        Ok(value) => {
            // 取值记审计：谁、哪个应用的哪个密码；值本身不进审计。
            if let Some(audit) = &gateway.audit {
                audit.record(&serde_json::json!({
                    "event": "client_password_read",
                    "plugin": plugin,
                    "id": id,
                    "by": acted_by(&gateway, peer.as_ref(), &headers),
                }));
            }
            no_store(
                Json(PasswordValue {
                    username: gateway.plugins.owner_username(),
                    password: SecretString::new(value),
                })
                .into_response(),
            )
        }
        Err(PasswordError::NotFound) => failure(
            ClientsFailure::InvalidInput,
            &l10n::text(&langs, "clients-password-not-found", &[]),
        ),
        Err(error) => storage_failure(&langs, error),
    }
}

/// `DELETE`：吊销；下一个请求就失效，已升级的连接立即断。
async fn revoke(
    State(gateway): State<Arc<Gateway>>,
    headers: HeaderMap,
    Path((plugin, id)): Path<(String, String)>,
) -> Response {
    let langs = languages(&headers);
    let Some(store) = &gateway.plugins.client_passwords else {
        return unavailable(&langs);
    };
    match store.revoke(&plugin, &id) {
        Ok(()) => StatusCode::OK.into_response(),
        Err(PasswordError::NotFound) => failure(
            ClientsFailure::InvalidInput,
            &l10n::text(&langs, "clients-password-not-found", &[]),
        ),
        Err(error) => storage_failure(&langs, error),
    }
}

/// 密码库读不出来：如实回 503，具体原因写日志。
fn storage_failure(langs: &Languages, error: PasswordError) -> Response {
    tracing::error!("专用密码库读写失败：{error}");
    failure(
        ClientsFailure::PluginUnavailable,
        &l10n::text(langs, "clients-passwords-unavailable", &[]),
    )
}

#[cfg(test)]
#[path = "clients_passwords_tests.rs"]
mod clients_passwords_tests;
