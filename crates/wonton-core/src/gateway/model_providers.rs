//! 模型网关服务商 API：列出、测试和保存预设或自定义服务商。

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use hyper::{HeaderMap, StatusCode};
use serde::de::DeserializeOwned;
use wonton_protocol::{CustomModelProvider, ModelProviderKey};

use super::api::{install_error, languages, owner_denied};
use super::{Gateway, TunnelPeer};
use crate::l10n;
use crate::plugin::languages::Languages;
use crate::plugin::manager::InstallError;
use crate::plugin::model_providers::ProviderAction;

pub async fn list(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
) -> Response {
    if let Some(denied) = owner_denied(&gateway, peer, &headers) {
        return denied;
    }
    match gateway.plugins.model_providers(&languages(&headers)).await {
        Ok(providers) => Json(providers).into_response(),
        Err(error) => install_error(&error),
    }
}

pub async fn save(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: Bytes,
) -> Response {
    preset(&gateway, peer, &headers, &id, &body, ProviderAction::Save).await
}

pub async fn test(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: Bytes,
) -> Response {
    preset(&gateway, peer, &headers, &id, &body, ProviderAction::Test).await
}

async fn preset(
    gateway: &Gateway,
    peer: Option<Extension<TunnelPeer>>,
    headers: &HeaderMap,
    id: &str,
    body: &[u8],
    action: ProviderAction,
) -> Response {
    if let Some(denied) = owner_denied(gateway, peer, headers) {
        return denied;
    }
    let langs = languages(headers);
    let request: ModelProviderKey = match parse(body, &langs) {
        Ok(request) => request,
        Err(error) => return install_error(&error),
    };
    result(
        gateway
            .plugins
            .model_provider_key(id, request.key.expose(), action, &langs)
            .await,
    )
}

pub async fn custom_save(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    custom(&gateway, peer, &headers, &body, ProviderAction::Save).await
}

pub async fn custom_test(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    custom(&gateway, peer, &headers, &body, ProviderAction::Test).await
}

async fn custom(
    gateway: &Gateway,
    peer: Option<Extension<TunnelPeer>>,
    headers: &HeaderMap,
    body: &[u8],
    action: ProviderAction,
) -> Response {
    if let Some(denied) = owner_denied(gateway, peer, headers) {
        return denied;
    }
    let langs = languages(headers);
    let request: CustomModelProvider = match parse(body, &langs) {
        Ok(request) => request,
        Err(error) => return install_error(&error),
    };
    result(
        gateway
            .plugins
            .custom_model_provider(&request, action, &langs)
            .await,
    )
}

fn parse<T: DeserializeOwned>(body: &[u8], langs: &Languages) -> Result<T, InstallError> {
    // serde 的类型错误可能包含输入值，不能把含 key 的请求片段带回客户端。
    serde_json::from_slice(body).map_err(|_| {
        InstallError::Invalid(l10n::text(
            langs,
            "plugin-settings-model-request-format",
            &[],
        ))
    })
}

fn result(result: Result<(), InstallError>) -> Response {
    match result {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => install_error(&error),
    }
}
