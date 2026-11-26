//! 专用密码的管理接口（ADR-0021「在 App 里」）：挂在 [`super::clients_api`] 的中间件后面，
//! 只有主人（令牌或已配对设备）能调。值每次取都记审计、具体到设备。
//!
//! 签发、换值、吊销都先落盘再跑应用里的命令：命令成功才算数，插件没在跑回 202、命令失败回 502，
//! 记录留在待完成，插件下次启动重试，值在此之前照旧能用。

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
use crate::clients::credentials::CredentialError;
use crate::clients::passwords::PasswordError;
use crate::clients::pending::{Issue, IssueError, Settle};
use crate::l10n;
use crate::plugin::languages::Languages;
use wonton_protocol::ClientsFailure;

/// 回给客户端的登录名：清单模板里的 `{user}` 换主人、`{email}` 换主人派生出的内部邮箱；
/// 没写就是主人。
fn credential_username(
    credentials: &wonton_protocol::manifest::CredentialsAuth,
    owner: &str,
) -> String {
    credentials
        .username
        .clone()
        .unwrap_or_else(|| "{user}".to_owned())
        .replace("{user}", owner)
        .replace("{email}", &crate::owner::internal_email(owner))
}

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

/// `POST`：写真值并跑应用的签发命令。成功 `201`、插件没在跑 `202`、命令失败 `502`（值都已落盘）。
async fn create(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path(plugin): Path<String>,
    Json(request): Json<NewPasswordRequest>,
) -> Response {
    let langs = languages(&headers);
    let Some(_store) = &gateway.plugins.client_passwords else {
        return unavailable(&langs);
    };
    let Ok(id) = crate::plugin::PluginId::parse(&plugin) else {
        return failure(
            ClientsFailure::InvalidInput,
            &l10n::text(&langs, "clients-password-not-found", &[]),
        );
    };
    if !gateway.plugins.is_installed(&id) {
        return failure(
            ClientsFailure::PluginUnavailable,
            &l10n::text(&langs, "clients-plugin-unavailable", &[]),
        );
    }
    // 设备身份：经隧道的已配对设备记 ID（吊销按它连带），名字给界面显示。
    let device = peer.as_ref().map(|Extension(peer)| {
        let name = gateway
            .tunnel
            .devices
            .get(&peer.0)
            .and_then(|device| device.name);
        (peer.0.to_string(), name)
    });
    let username = gateway
        .plugins
        .credentials_auth(&id)
        .map(|credentials| credential_username(&credentials, &gateway.plugins.owner_username()))
        .unwrap_or_else(|| gateway.plugins.owner_username());
    let outcome = gateway
        .plugins
        .issue_new(
            &id,
            &request.label,
            device
                .as_ref()
                .map(|(id, name)| (id.as_str(), name.as_deref())),
        )
        .await;
    match outcome {
        Ok(Issue::Done(issued)) => issued_response(
            &gateway,
            &plugin,
            &username,
            peer.as_ref(),
            &headers,
            issued,
            false,
        ),
        Ok(Issue::Deferred(issued)) => issued_response(
            &gateway,
            &plugin,
            &username,
            peer.as_ref(),
            &headers,
            issued,
            true,
        ),
        Ok(Issue::Failed { issued, error }) => {
            if let Some(audit) = &gateway.audit {
                audit.record(&serde_json::json!({
                    "event": "client_password_issue_failed",
                    "plugin": plugin,
                    "id": &issued.id,
                    "error": error.detail(),
                    "by": acted_by(&gateway, peer.as_ref(), &headers),
                }));
            }
            let mut response = credential_failure(&langs, error);
            response.headers_mut().insert(
                wonton_protocol::PASSWORD_ID_HEADER,
                issued.id.parse().expect("记录 ID 是 ASCII"),
            );
            no_store(response)
        }
        Err(IssueError::Store) => unavailable(&langs),
        Err(IssueError::SharedExists) => failure(
            ClientsFailure::Conflict,
            &l10n::text(&langs, "clients-credentials-shared-exists", &[]),
        ),
        Err(IssueError::Password(PasswordError::InvalidLabel)) => failure(
            ClientsFailure::InvalidInput,
            &l10n::text(&langs, "clients-password-label-invalid", &[]),
        ),
        Err(IssueError::Password(error)) => storage_failure(&langs, error),
    }
}

fn issued_response(
    gateway: &Gateway,
    plugin: &str,
    username: &str,
    peer: Option<&Extension<TunnelPeer>>,
    headers: &HeaderMap,
    issued: crate::clients::pending::Issued,
    deferred: bool,
) -> Response {
    if let Some(audit) = &gateway.audit {
        audit.record(&serde_json::json!({
            "event": "client_password_issue",
            "plugin": plugin,
            "id": issued.id,
            "pending": deferred,
            "by": acted_by(gateway, peer, headers),
        }));
    }
    let status = if deferred {
        StatusCode::ACCEPTED
    } else {
        StatusCode::CREATED
    };
    let value_pending = deferred && issued.value.is_empty();
    let response = (
        status,
        Json(PasswordValue {
            username: username.to_owned(),
            password: SecretString::new(issued.value),
        }),
    )
        .into_response();
    if value_pending {
        // `generates` 的待换：值还在应用那边，别让客户端把空串当值。
        return with_pending(response, wonton_protocol::PendingOp::Rotate);
    }
    no_store(response)
}

/// `GET`：取值；每次都记审计、具体到设备。待完成的记录用响应头标出来，值照旧给。
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
            let pending = store
                .snapshot(&plugin, &id)
                .ok()
                .and_then(|snapshot| crate::plugin::PluginManager::pending_op(&snapshot.state));
            let username = crate::plugin::PluginId::parse(&plugin)
                .ok()
                .and_then(|id| gateway.plugins.credentials_auth(&id))
                .map(|credentials| {
                    credential_username(&credentials, &gateway.plugins.owner_username())
                })
                .unwrap_or_else(|| gateway.plugins.owner_username());
            let response = Json(PasswordValue {
                username,
                password: SecretString::new(value),
            })
            .into_response();
            match pending {
                Some(pending) => with_pending(response, pending),
                None => no_store(response),
            }
        }
        Err(PasswordError::NotFound) => failure(
            ClientsFailure::InvalidInput,
            &l10n::text(&langs, "clients-password-not-found", &[]),
        ),
        Err(error) => storage_failure(&langs, error),
    }
}

/// `DELETE /_wonton/clients/{plugin}/passwords/{id}`：吊销；`per_client: false` 时换成新值。
/// 完成 `200`，插件没在跑 `202`，失败 `502`；只有 `202` 带
/// [`PENDING_HEADER`](wonton_protocol::PASSWORD_PENDING_HEADER) 标出没做完的吊销或换值。吊销设备走 `devices.rs`，不经过这里。
async fn revoke(
    State(gateway): State<Arc<Gateway>>,
    headers: HeaderMap,
    Path((plugin, id)): Path<(String, String)>,
) -> Response {
    let langs = languages(&headers);
    let Some(_store) = &gateway.plugins.client_passwords else {
        return unavailable(&langs);
    };
    let Ok(plugin_id) = crate::plugin::PluginId::parse(&plugin) else {
        return failure(
            ClientsFailure::InvalidInput,
            &l10n::text(&langs, "clients-password-not-found", &[]),
        );
    };
    if !gateway.plugins.is_installed(&plugin_id) {
        return failure(
            ClientsFailure::PluginUnavailable,
            &l10n::text(&langs, "clients-plugin-unavailable", &[]),
        );
    }
    match gateway.plugins.settle_password(&plugin_id, &id).await {
        Ok(Settle::Done) => no_store(StatusCode::OK.into_response()),
        Ok(Settle::Deferred(op)) => with_pending(StatusCode::ACCEPTED.into_response(), op),
        Ok(Settle::Failed(error)) => credential_failure(&langs, error),
        Err(PasswordError::NotFound) => failure(
            ClientsFailure::InvalidInput,
            &l10n::text(&langs, "clients-password-not-found", &[]),
        ),
        Err(error) => storage_failure(&langs, error),
    }
}

/// 标出还没做完的操作，值照旧照给。
fn with_pending(mut response: Response, op: wonton_protocol::PendingOp) -> Response {
    let marker = match op {
        wonton_protocol::PendingOp::Revoke => "revoke",
        wonton_protocol::PendingOp::Rotate => "rotate",
        wonton_protocol::PendingOp::Other => "other",
    };
    response.headers_mut().insert(
        wonton_protocol::PASSWORD_PENDING_HEADER,
        marker.parse().expect("静态标记"),
    );
    no_store(response)
}

/// 应用里的命令失败：`502`，带标准错误输出；超时说明可能已经生效，会重试。
fn credential_failure(langs: &Languages, error: CredentialError) -> Response {
    match error {
        CredentialError::Timeout => failure(
            ClientsFailure::CredentialsFailed,
            &l10n::text(langs, "clients-credentials-timeout", &[]),
        ),
        CredentialError::Unavailable => failure(
            ClientsFailure::PluginUnavailable,
            &l10n::text(langs, "clients-credentials-unavailable", &[]),
        ),
        CredentialError::Missing(op) => failure(
            ClientsFailure::CredentialsFailed,
            &l10n::text(
                langs,
                "clients-credentials-missing-command",
                &[("op", format!("{op:?}").to_lowercase().into())],
            ),
        ),
        CredentialError::Failed { stderr, .. } => failure(
            ClientsFailure::CredentialsFailed,
            &l10n::text(
                langs,
                "clients-credentials-failed",
                &[("error", stderr.into())],
            ),
        ),
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
#[path = "clients_passwords_tests/mod.rs"]
mod clients_passwords_tests;
