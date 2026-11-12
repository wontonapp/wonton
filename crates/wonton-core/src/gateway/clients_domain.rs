//! 自己的域名设置（ADR-0089）：GET 看状态，PUT 写后缀、服务商和只写不读的凭据，GET 服务商预设。
//!
//! `PUT` 成功后当场回新的状态并排一次签发；后台按 `prepare` → `issue` 跑，失败保留旧证书。

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use hyper::HeaderMap;
use hyper::header::HeaderValue;
use wonton_protocol::{DnsProviderPreset, PutDomainRequest};

use super::Gateway;
use super::api::languages;
use super::clients_api::{failure, no_store, unavailable};
use crate::clients::domain::{PREVIOUS_SUFFIX_HEADER, SUFFIX_CHANGED_HEADER};
use crate::clients::providers;
use crate::l10n;
use wonton_protocol::ClientsFailure;

/// `GET /_wonton/clients/domain`
pub async fn domain(State(gateway): State<Arc<Gateway>>, headers: HeaderMap) -> Response {
    let langs = languages(&headers);
    let Some(clients) = &gateway.clients else {
        return unavailable(&langs);
    };
    match clients.view().await {
        Ok(view) => no_store(Json(view).into_response()),
        Err(error) => failure(
            ClientsFailure::InvalidInput,
            &l10n::text(
                &langs,
                "clients-domain-read-failed",
                &[("error", format!("{error:#}").into())],
            ),
        ),
    }
}

/// `PUT /_wonton/clients/domain`
pub async fn save_domain(
    State(gateway): State<Arc<Gateway>>,
    headers: HeaderMap,
    Json(request): Json<PutDomainRequest>,
) -> Response {
    let langs = languages(&headers);
    let Some(clients) = &gateway.clients else {
        return unavailable(&langs);
    };
    let saved = match clients.save(request).await {
        Ok(saved) => saved,
        Err(crate::clients::domain::DomainError::Invalid(message)) => {
            return failure(ClientsFailure::InvalidInput, &message);
        }
        Err(crate::clients::domain::DomainError::Conflict(message)) => {
            return failure(ClientsFailure::Conflict, &message);
        }
    };
    let Ok(view) = clients.view().await else {
        return failure(
            ClientsFailure::InvalidInput,
            &l10n::text(&langs, "clients-domain-read-failed", &[]),
        );
    };
    let mut response = no_store(Json(view).into_response());
    if saved.suffix_changed {
        response.headers_mut().insert(
            SUFFIX_CHANGED_HEADER,
            HeaderValue::from_static("suffix_changed"),
        );
        if let Some(previous) = saved.previous_suffix
            && let Ok(value) = HeaderValue::from_str(&previous)
        {
            response.headers_mut().insert(PREVIOUS_SUFFIX_HEADER, value);
        }
    }
    response
}

/// `GET /_wonton/clients/domain/providers`
pub async fn providers(_headers: HeaderMap) -> Response {
    let presets: Vec<DnsProviderPreset> = providers::presets()
        .into_iter()
        .map(|preset| DnsProviderPreset {
            code: preset.code,
            name: preset.name,
            credential_vars: preset.credential_vars,
            signup_url: preset.signup_url,
            recommended: preset.recommended,
        })
        .collect();
    no_store(Json(presets).into_response())
}
