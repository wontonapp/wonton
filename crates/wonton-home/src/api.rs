use crate::{
    auth,
    service::{ApiError, App},
    store::{Caa, Home, hash, matches, now, random_label, random_token},
};
use axum::{
    Json,
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, Method, StatusCode, Uri},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

type Reply = Result<Response, ApiError>;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Register {
    invite: String,
}

pub async fn register(
    State(app): State<Arc<App>>,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Reply {
    let endpoint = auth::authenticate(&app, Method::POST, &uri, &headers, &body).await?;
    let request: Register =
        serde_json::from_slice(&body).map_err(|_| ApiError::bad("invalid registration body"))?;
    let mut store = app.store.lock().await;
    let existing = store
        .data
        .homes
        .iter()
        .find(|(_, home)| home.endpoint == endpoint)
        .map(|(label, _)| label.clone());
    let (label, created) = if let Some(label) = existing {
        let home = &store.data.homes[&label];
        if home.suspended || home.deleted || !matches(&request.invite, &home.invite_hash) {
            return Err(ApiError::forbidden());
        }
        (label, false)
    } else {
        let invite_hash = hash(request.invite.as_bytes());
        if !store
            .data
            .invites
            .get(&invite_hash)
            .is_some_and(|expiry| *expiry >= now())
        {
            return Err(ApiError::forbidden());
        }
        let mut label = random_label()?;
        while store.data.homes.contains_key(&label) {
            label = random_label()?;
        }
        let home = Home {
            endpoint,
            invite_hash: invite_hash.clone(),
            token_hash: String::new(),
            acme_dns: app.dns.register_acme().await?,
            lan_acme_dns: app.dns.register_acme().await?,
            suspended: false,
            deleted: false,
            reason: None,
            caa: None,
            pending_caa: None,
            usage_month: 0,
            usage_bytes: 0,
        };
        store.data.invites.remove(&invite_hash);
        store.data.homes.insert(label.clone(), home);
        // Consume the invite and reserve the permanent label before making DNS visible.
        store.save()?;
        (label, true)
    };
    let token = random_token()?;
    store
        .data
        .homes
        .get_mut(&label)
        .expect("reserved home")
        .token_hash = hash(token.as_bytes());
    store.save()?;
    let home = &store.data.homes[&label];
    app.dns.reconcile(&label, home).await?;
    Ok((
        if created {
            StatusCode::CREATED
        } else {
            StatusCode::OK
        },
        Json(json!({
            "home":label,"domain":format!("{label}.{}",app.config.zone),"frp_token":token,
            "acme_dns":home.acme_dns,"lan_acme_dns":home.lan_acme_dns,
            "acme_dns_url":app.config.acme_url,"relay":app.config.relay,"suspended":false,
        })),
    )
        .into_response())
}

fn owned<'a>(
    store: &'a mut crate::store::Store,
    label: &str,
    endpoint: &str,
) -> Result<&'a mut Home, ApiError> {
    store
        .data
        .homes
        .get_mut(label)
        .filter(|home| home.endpoint == endpoint && !home.deleted)
        .ok_or_else(ApiError::forbidden)
}

pub async fn status(
    State(app): State<Arc<App>>,
    Path(label): Path<String>,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Reply {
    let endpoint = auth::authenticate(&app, Method::GET, &uri, &headers, &body).await?;
    if !body.is_empty() {
        return Err(ApiError::bad("GET body must be empty"));
    }
    let mut store = app.store.lock().await;
    let home = owned(&mut store, &label, &endpoint)?;
    Ok(Json(json!({"home":label,"domain":format!("{label}.{}",app.config.zone),"suspended":home.suspended,"reason":home.reason})).into_response())
}

pub async fn delete(
    State(app): State<Arc<App>>,
    Path(label): Path<String>,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Reply {
    let endpoint = auth::authenticate(&app, Method::DELETE, &uri, &headers, &body).await?;
    if !body.is_empty() {
        return Err(ApiError::bad("DELETE body must be empty"));
    }
    let mut store = app.store.lock().await;
    let home = owned(&mut store, &label, &endpoint)?;
    home.deleted = true;
    home.suspended = true;
    home.reason = Some("deleted".into());
    store.save()?;
    app.dns.reconcile(&label, &store.data.homes[&label]).await?;
    Ok(Json(json!({})).into_response())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CaaRequest {
    account_uri: String,
}

pub async fn caa(
    State(app): State<Arc<App>>,
    Path(label): Path<String>,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Reply {
    let endpoint = auth::authenticate(&app, Method::PUT, &uri, &headers, &body).await?;
    let request: CaaRequest =
        serde_json::from_slice(&body).map_err(|_| ApiError::bad("invalid CAA body"))?;
    let account =
        url::Url::parse(&request.account_uri).map_err(|_| ApiError::bad("invalid account URI"))?;
    if account.scheme() != "https"
        || account.host_str().is_none()
        || !account.username().is_empty()
        || account.password().is_some()
        || account.fragment().is_some()
        || request
            .account_uri
            .bytes()
            .any(|b| !b.is_ascii_graphic() || b == b'"' || b == b'\\' || b == b';')
    {
        return Err(ApiError::bad("CAA issuer or account URI forbidden"));
    }
    let mut store = app.store.lock().await;
    let home = owned(&mut store, &label, &endpoint)?;
    if home.suspended {
        return Err(ApiError::forbidden());
    }
    let clock = now();
    let effective = match home.caa.as_ref() {
        None => {
            home.caa = Some(Caa {
                account_uri: request.account_uri,
                effective_at: clock,
            });
            true
        }
        Some(current) if current.account_uri == request.account_uri => {
            home.pending_caa = None;
            true
        }
        Some(_) => {
            if !home
                .pending_caa
                .as_ref()
                .is_some_and(|pending| pending.account_uri == request.account_uri)
            {
                home.pending_caa = Some(Caa {
                    account_uri: request.account_uri,
                    effective_at: clock + 86400,
                });
            }
            false
        }
    };
    store.save()?;
    app.dns.reconcile(&label, &store.data.homes[&label]).await?;
    let home = &store.data.homes[&label];
    Ok((
        if effective {
            StatusCode::OK
        } else {
            StatusCode::ACCEPTED
        },
        Json(json!({"effective":effective,"pending":home.pending_caa})),
    )
        .into_response())
}

pub async fn acme_update(
    State(app): State<Arc<App>>,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Reply {
    if uri.query().is_some() {
        return Err(ApiError::bad("query strings are forbidden"));
    }
    let user = headers
        .get("x-api-user")
        .and_then(|h| h.to_str().ok())
        .ok_or_else(ApiError::forbidden)?;
    let key = headers
        .get("x-api-key")
        .and_then(|h| h.to_str().ok())
        .ok_or_else(ApiError::forbidden)?;
    let request: serde_json::Value =
        serde_json::from_slice(&body).map_err(|_| ApiError::bad("invalid update"))?;
    let store = app.store.lock().await;
    let allowed = store
        .data
        .homes
        .values()
        .filter(|h| !h.deleted && !h.suspended)
        .any(|home| {
            [&home.acme_dns, &home.lan_acme_dns].iter().any(|account| {
                account.username == user && matches(key, &hash(account.password.as_bytes()))
            })
        });
    if !allowed {
        return Err(ApiError::forbidden());
    }
    let upstream = app
        .client
        .post(format!(
            "{}/update",
            app.config.acme_internal.trim_end_matches('/')
        ))
        .header("X-Api-User", user)
        .header("X-Api-Key", key)
        .json(&request)
        .send()
        .await
        .map_err(anyhow::Error::from)?;
    let status = upstream.status();
    let bytes = upstream.bytes().await.map_err(anyhow::Error::from)?;
    Ok((status, [("content-type", "application/json")], bytes).into_response())
}
