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
use wonton_protocol::{
    ClientAuthKind, ClientsPlugin, EntryBlock, EntryKind, EntrySwitch, NewPasswordRequest,
    PasswordSummary, PasswordValue, PublicRuleMode, PublicRules, SecretString,
};

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

/// `GET /_wonton/clients` 的响应：全部客户端的入口和密码。
type ClientsList = Vec<ClientsPlugin>;

/// 回给客户端的登录名：清单模板里的 `{user}` 换主人；没写就是主人。
fn credential_username(
    credentials: &wonton_protocol::manifest::CredentialsAuth,
    owner: &str,
) -> String {
    credentials
        .username
        .clone()
        .unwrap_or_else(|| "{user}".to_owned())
        .replace("{user}", owner)
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

/// `GET`：每个声明了 `clients.http` 的插件的入口开关、密码清单和现在开不了的原因。
pub(super) async fn list(State(gateway): State<Arc<Gateway>>, headers: HeaderMap) -> Response {
    let langs = languages(&headers);
    let store = gateway.plugins.client_passwords.as_ref();
    let views = match gateway.plugins.client_views() {
        Ok(views) => views,
        Err(error) => {
            tracing::error!("读插件表失败：{error}");
            return unavailable(&langs);
        }
    };
    let owner = gateway.plugins.owner_username();
    let mut plugins: ClientsList = Vec::with_capacity(views.len());
    for view in views {
        let credentials = view.route.http.auth.credentials.clone();
        let gateway_auth = view.route.http.auth.gateway.is_some();
        let auth = if credentials.is_some() {
            ClientAuthKind::Credentials
        } else if gateway_auth {
            ClientAuthKind::Gateway
        } else {
            ClientAuthKind::App
        };
        let username = if let Some(credentials) = &credentials {
            Some(credential_username(credentials, &owner))
        } else if gateway_auth {
            Some(owner.clone())
        } else {
            None
        };
        let passwords = store
            .map(|store| {
                store
                    .snapshots(view.id.as_str())
                    .into_iter()
                    .map(|snapshot| PasswordSummary {
                        pending: crate::plugin::PluginManager::pending_op(&snapshot.state),
                        id: snapshot.id,
                        label: snapshot.label,
                        created_by_device: snapshot.created_by_device,
                        last_viewed_by: {
                            let mut viewed: Vec<_> = snapshot.viewed_by.into_iter().collect();
                            viewed.sort_by_key(|(_, at)| std::cmp::Reverse(*at));
                            viewed.into_iter().map(|(device, _)| device).collect()
                        },
                    })
                    .collect()
            })
            .unwrap_or_default();
        let blocked = credentials
            .as_ref()
            .filter(|_| !gateway.plugins.credentials_ready(&view.id))
            .map(|_| EntryBlock::CredentialsNotIssued);
        plugins.push(ClientsPlugin {
            id: view.id.to_string(),
            label: view.label.clone(),
            lan_url: view.switches.lan.then(|| {
                gateway.public.origin(&format!(
                    "{}.lan.{}",
                    view.id.slug(),
                    gateway.public.base_domain
                ))
            }),
            public_url: view.switches.public.then(|| {
                gateway
                    .public
                    .origin(&gateway.public.plugin_host(&view.id.slug()))
            }),
            entries: entry_switches(&view),
            auth,
            username,
            passwords,
            public_rules: view.route.http.public.as_ref().map(|public| PublicRules {
                mode: if public.allow.is_some() {
                    PublicRuleMode::Allow
                } else {
                    PublicRuleMode::Deny
                },
                paths: public
                    .allow
                    .clone()
                    .or_else(|| public.deny.clone())
                    .unwrap_or_default(),
            }),
            blocked,
        });
    }
    no_store(Json(plugins).into_response())
}

fn entry_switches(view: &crate::plugin::manager::ClientsView) -> Vec<EntrySwitch> {
    let mut entries: Vec<EntrySwitch> = [
        (EntryKind::Lan, view.switches.lan),
        (EntryKind::Public, view.switches.public),
    ]
    .into_iter()
    .map(|(kind, enabled)| EntrySwitch { kind, enabled })
    .collect();
    let mut tcp: Vec<EntrySwitch> = view
        .tcp_names
        .iter()
        .map(|name| EntrySwitch {
            kind: EntryKind::Tcp { name: name.clone() },
            enabled: false,
        })
        .collect();
    tcp.sort_by_key(|switch| match &switch.kind {
        EntryKind::Tcp { name } => name.clone(),
        _ => String::new(),
    });
    entries.extend(tcp);
    entries
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
                    "id": issued.id,
                    "error": error.detail(),
                    "by": acted_by(&gateway, peer.as_ref(), &headers),
                }));
            }
            credential_failure(&langs, error)
        }
        Err(IssueError::Store) => unavailable(&langs),
        Err(IssueError::NotCredentials) => failure(
            ClientsFailure::InvalidInput,
            &l10n::text(&langs, "clients-credentials-missing", &[]),
        ),
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
    no_store(
        (
            status,
            Json(PasswordValue {
                username: username.to_owned(),
                password: SecretString::new(issued.value),
            }),
        )
            .into_response(),
    )
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
            let mut response = Json(PasswordValue {
                username,
                password: SecretString::new(value),
            })
            .into_response();
            if let Some(pending) = pending {
                let marker = match pending {
                    wonton_protocol::PendingOp::Revoke => "revoke",
                    wonton_protocol::PendingOp::Rotate => "rotate",
                    wonton_protocol::PendingOp::Other => "other",
                };
                response.headers_mut().insert(
                    wonton_protocol::PASSWORD_PENDING_HEADER,
                    marker.parse().expect("静态标记"),
                );
            }
            no_store(response)
        }
        Err(PasswordError::NotFound) => failure(
            ClientsFailure::InvalidInput,
            &l10n::text(&langs, "clients-password-not-found", &[]),
        ),
        Err(error) => storage_failure(&langs, error),
    }
}

/// `DELETE`：吊销；`per_client: false` 时换成新值。完成 `200`，插件没在跑 `202`，失败 `502`。
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
    match gateway.plugins.settle_password(&plugin_id, &id).await {
        Ok(Settle::Done) => StatusCode::OK.into_response(),
        Ok(Settle::Deferred) => StatusCode::ACCEPTED.into_response(),
        Ok(Settle::Failed(error)) => credential_failure(&langs, error),
        Err(PasswordError::NotFound) => failure(
            ClientsFailure::InvalidInput,
            &l10n::text(&langs, "clients-password-not-found", &[]),
        ),
        Err(error) => storage_failure(&langs, error),
    }
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
#[path = "clients_passwords_tests.rs"]
mod clients_passwords_tests;
