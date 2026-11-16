//! Friend OIDC is a code flow for one currently shared plugin, not general Hydra access.
//! Verifiers returned by Hydra are one-use capabilities bound to this gateway, friend generation
//! and client. They may continue only through GET /oauth2/auth; browser token APIs stay closed.

use std::collections::HashMap;
use std::sync::Mutex;

use axum::extract::Request;
use hyper::{Method, Uri};
use wonton_protocol::{GatewayRefusal, Role};

use super::{AccessGuard, SharePeer, guard_request};
use crate::gateway::{Gateway, Person};

const CONTINUATION_TTL_MS: u64 = 10 * 60 * 1000;
const MAX_CONTINUATIONS: usize = 4096;

type Refusal = GatewayRefusal;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::gateway) enum Step {
    Login,
    Consent,
}

#[derive(Debug, PartialEq, Eq)]
enum AuthRoute {
    Metadata,
    Authorize {
        client: String,
        continuation: Option<(Step, String)>,
    },
}

#[derive(Debug)]
struct Binding {
    user_id: String,
    generation: u64,
    client: String,
    step: Step,
    expires_at_ms: u64,
    permits: Vec<crate::shares::GrantPermit>,
}

#[derive(Default)]
pub struct Continuations(Mutex<HashMap<String, Binding>>);

impl Continuations {
    fn insert(
        &self,
        peer: &SharePeer,
        client: &str,
        step: Step,
        verifier: String,
        now: u64,
        permits: Vec<crate::shares::GrantPermit>,
    ) -> Result<(), Refusal> {
        let mut entries = self.0.lock().expect("OIDC continuation lock poisoned");
        entries.retain(|_, binding| now < binding.expires_at_ms);
        if entries.len() >= MAX_CONTINUATIONS || entries.contains_key(&verifier) {
            return Err(Refusal::ShareRequired);
        }
        entries.insert(
            verifier,
            Binding {
                user_id: peer.user_id.clone(),
                generation: peer.generation,
                client: client.to_owned(),
                step,
                expires_at_ms: now.saturating_add(CONTINUATION_TTL_MS),
                permits,
            },
        );
        Ok(())
    }

    /// 校验都在这一个口子上，参数多一点好过让调用方拼结构体。
    #[allow(clippy::too_many_arguments)]
    fn check(
        &self,
        peer: &SharePeer,
        client: &str,
        step: Step,
        verifier: &str,
        now: u64,
        permits: &[crate::shares::GrantPermit],
        consume: bool,
    ) -> Result<(), Refusal> {
        let mut entries = self.0.lock().expect("OIDC continuation lock poisoned");
        entries.retain(|_, binding| now < binding.expires_at_ms);
        let valid = entries.get(verifier).is_some_and(|binding| {
            binding.user_id == peer.user_id
                && binding.generation == peer.generation
                && binding.client == client
                && binding.step == step
                && binding
                    .permits
                    .iter()
                    .any(|permit| permits.contains(permit))
        });
        if !valid {
            return Err(Refusal::ShareRequired);
        }
        if consume {
            entries.remove(verifier);
        }
        Ok(())
    }
}

fn auth_route(method: &Method, uri: &Uri) -> Result<AuthRoute, Refusal> {
    if method != Method::GET {
        return Err(Refusal::ShareRequired);
    }
    match uri.path() {
        crate::oidc::DISCOVERY_PATH | "/.well-known/jwks.json" => return Ok(AuthRoute::Metadata),
        "/oauth2/auth" => {}
        _ => return Err(Refusal::ShareRequired),
    }
    let mut params = HashMap::new();
    for (key, value) in url::form_urlencoded::parse(uri.query().unwrap_or_default().as_bytes()) {
        if params.insert(key, value).is_some() {
            return Err(Refusal::ShareRequired);
        }
    }
    // Request objects/PAR can hide a different client or response type from this gateway.
    if ["request", "request_uri", "device_verifier"]
        .iter()
        .any(|key| params.contains_key(*key))
    {
        return Err(Refusal::ShareRequired);
    }
    let client = params
        .get("client_id")
        .filter(|id| !id.is_empty())
        .ok_or(Refusal::ShareRequired)?
        .to_string();
    let login = params.get("login_verifier");
    let consent = params.get("consent_verifier");
    let continuation = match (login, consent) {
        (Some(value), None) if !value.is_empty() => Some((Step::Login, value.to_string())),
        (None, Some(value)) if !value.is_empty() => Some((Step::Consent, value.to_string())),
        (None, None) => {
            if params.get("response_type").map(|v| v.as_ref()) != Some("code") {
                return Err(Refusal::ShareRequired);
            }
            None
        }
        _ => return Err(Refusal::ShareRequired),
    };
    Ok(AuthRoute::Authorize {
        client,
        continuation,
    })
}

/// The dispatcher may use the returned plugin ID to attach its request/stream guard.
/// Auth forwarding also performs this check itself; discovery/JWKS require an active friend only.
pub(in crate::gateway) fn authorize_auth(
    gateway: &Gateway,
    peer: &SharePeer,
    method: &Method,
    uri: &Uri,
) -> Result<Option<String>, Refusal> {
    let sharing = gateway.sharing.as_ref().ok_or(Refusal::ShareRequired)?;
    sharing.person(peer)?;
    match auth_route(method, uri)? {
        AuthRoute::Metadata => Ok(None),
        AuthRoute::Authorize {
            client,
            continuation,
        } => {
            if !gateway.plugins.shared_oidc_client(&client) {
                return Err(Refusal::ShareRequired);
            }
            let access = sharing.authorize(peer, &client, &gateway.plugins)?;
            if let Some((step, verifier)) = continuation {
                sharing.oidc.check(
                    peer,
                    &client,
                    step,
                    &verifier,
                    sharing.now(),
                    &access.permits,
                    false,
                )?;
            }
            Ok(Some(client))
        }
    }
}

pub(in crate::gateway) fn consume_auth(
    gateway: &Gateway,
    peer: &SharePeer,
    method: &Method,
    uri: &Uri,
) -> Result<(), Refusal> {
    if let AuthRoute::Authorize {
        client,
        continuation: Some((step, verifier)),
    } = auth_route(method, uri)?
    {
        let sharing = gateway.sharing.as_ref().ok_or(Refusal::ShareRequired)?;
        let access = sharing.authorize(peer, &client, &gateway.plugins)?;
        sharing.oidc.check(
            peer,
            &client,
            step,
            &verifier,
            sharing.now(),
            &access.permits,
            true,
        )?;
    }
    Ok(())
}

/// Presence of SharePeer is authoritative even when the owner cookie/device also authenticates.
pub(in crate::gateway) fn person(
    gateway: &Gateway,
    req: &Request,
) -> Result<Option<Person>, Refusal> {
    if let Some(peer) = req.extensions().get::<SharePeer>() {
        return gateway
            .sharing
            .as_ref()
            .ok_or(Refusal::ShareRequired)?
            .person(peer)
            .map(Some);
    }
    Ok(crate::gateway::bot_web::core_person(gateway, req))
}

pub(in crate::gateway) fn authorize_challenge(
    gateway: &Gateway,
    req: &mut Request,
    client: &str,
) -> Result<Option<Person>, Refusal> {
    let Some(peer) = req.extensions().get::<SharePeer>() else {
        return Ok(None);
    };
    let sharing = gateway.sharing.as_ref().ok_or(Refusal::ShareRequired)?;
    if !gateway.plugins.shared_oidc_client(client) {
        return Err(Refusal::ShareRequired);
    }
    let access = sharing.authorize(peer, client, &gateway.plugins)?;
    let person = Person {
        username: access.friend.username,
        role: Role::Friend,
    };
    guard_request(gateway, req, client)?;
    Ok(Some(person))
}

pub(in crate::gateway) async fn while_active<T>(
    guard: Option<AccessGuard>,
    future: impl Future<Output = T>,
) -> Result<T, Refusal> {
    if let Some(guard) = guard {
        tokio::select! {
            biased;
            _ = guard.cancelled() => Err(Refusal::ShareRevoked),
            result = future => Ok(result),
        }
    } else {
        Ok(future.await)
    }
}

fn continuation(origin: &str, target: &str, client: &str, step: Step) -> Result<String, Refusal> {
    let expected = url::Url::parse(origin).map_err(|_| Refusal::ShareRequired)?;
    let target = url::Url::parse(target).map_err(|_| Refusal::ShareRequired)?;
    if target.origin() != expected.origin()
        || !target.username().is_empty()
        || target.password().is_some()
        || target.fragment().is_some()
    {
        return Err(Refusal::ShareRequired);
    }
    let uri: Uri = target
        .as_str()
        .parse()
        .map_err(|_| Refusal::ShareRequired)?;
    match auth_route(&Method::GET, &uri)? {
        AuthRoute::Authorize {
            client: found,
            continuation: Some((found_step, verifier)),
        } if found == client && found_step == step => Ok(verifier),
        _ => Err(Refusal::ShareRequired),
    }
}

pub(in crate::gateway) fn remember_redirect(
    gateway: &Gateway,
    req: &Request,
    client: &str,
    step: Step,
    target: &str,
) -> Result<(), Refusal> {
    let Some(peer) = req.extensions().get::<SharePeer>() else {
        return Ok(());
    };
    let sharing = gateway.sharing.as_ref().ok_or(Refusal::ShareRequired)?;
    // Acceptance involved an await: revoke, expiry, removal and uninstall must be checked again.
    let mut access = sharing.authorize(peer, client, &gateway.plugins)?;
    let guard = req
        .extensions()
        .get::<AccessGuard>()
        .ok_or(Refusal::ShareRevoked)?;
    if !guard.allows(&access) {
        return Err(Refusal::ShareRevoked);
    }
    access
        .permits
        .retain(|permit| guard.permits().contains(permit));
    let origin = gateway.public.origin(&gateway.public.auth_host());
    let verifier = continuation(&origin, target, client, step)?;
    sharing
        .oidc
        .insert(peer, client, step, verifier, sharing.now(), access.permits)
}

#[cfg(test)]
#[path = "oidc_tests.rs"]
mod tests;
