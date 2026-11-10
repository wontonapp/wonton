//! Connection-scoped sharing identity; never accepted from HTTP headers or cookies.
pub(super) mod api;
mod guard;
mod handler;
pub(super) mod oidc;
mod request;

pub(super) use guard::{AccessGuard, StreamAccess, guard_body, guard_request};
pub use handler::SharingHandler;

use super::{Gateway, Person, pages};
use crate::plugin::{PluginManager, languages::Languages};
use crate::{
    friends::Friends,
    share_identity::IdentityVerifier,
    shares::{Access, Shares},
};
use axum::response::Response;
use hyper::StatusCode;
use std::sync::Arc;
use wonton_protocol::{GatewayRefusal, Role};

pub struct Sharing {
    pub friends: Arc<Friends>,
    pub shares: Arc<Shares>,
    pub identity: Arc<IdentityVerifier>,
    pub(super) oidc: oidc::Continuations,
    clock: Arc<dyn Fn() -> u64 + Send + Sync>,
}

#[derive(Clone, Debug)]
pub(super) struct SharePeer {
    pub user_id: String,
    pub generation: u64,
}

impl Sharing {
    pub fn new(
        friends: Arc<Friends>,
        shares: Arc<Shares>,
        identity: Arc<IdentityVerifier>,
    ) -> Self {
        Self {
            friends,
            shares,
            identity,
            oidc: Default::default(),
            clock: Arc::new(now_ms),
        }
    }

    pub(super) fn now(&self) -> u64 {
        (self.clock)()
    }

    #[cfg(test)]
    fn with_clock(mut self, clock: Arc<dyn Fn() -> u64 + Send + Sync>) -> Self {
        self.clock = clock;
        self
    }

    pub(super) fn person(&self, peer: &SharePeer) -> Result<Person, GatewayRefusal> {
        let friend = self
            .friends
            .get(&peer.user_id)
            .ok_or(GatewayRefusal::ShareRevoked)?;
        if friend.access_generation != peer.generation {
            return Err(GatewayRefusal::ShareRevoked);
        }
        Ok(Person {
            username: friend.username,
            role: Role::Friend,
        })
    }

    pub(super) fn authorize(
        &self,
        peer: &SharePeer,
        plugin_id: &str,
        plugins: &PluginManager,
    ) -> Result<Access, GatewayRefusal> {
        let access = self
            .shares
            .evaluate(&peer.user_id, plugin_id, self.now())
            .map_err(|reason| match reason {
                crate::shares::GatewayRefusal::ShareRequired => GatewayRefusal::ShareRequired,
                crate::shares::GatewayRefusal::ShareRevoked => GatewayRefusal::ShareRevoked,
                crate::shares::GatewayRefusal::ShareExpired => GatewayRefusal::ShareExpired,
            })?;
        if access.friend.access_generation != peer.generation {
            return Err(GatewayRefusal::ShareRevoked);
        }
        let id = crate::plugin::manifest::PluginId::parse(plugin_id)
            .map_err(|_| GatewayRefusal::ShareRequired)?;
        if !plugins.share_offer(&id).installed {
            return Err(GatewayRefusal::ShareRevoked);
        }
        Ok(access)
    }
}

pub(super) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis().min(u128::from(u64::MAX)) as u64)
}

pub(super) fn refused(
    reason: GatewayRefusal,
    langs: &Languages,
    format: crate::gateway::pages::Format,
) -> Response {
    let kind = match reason {
        GatewayRefusal::ShareIdentity => "share_identity",
        GatewayRefusal::ShareExpired => "share_expired",
        GatewayRefusal::ShareRevoked => "share_revoked",
        _ => "share_required",
    };
    let status = if reason == GatewayRefusal::ShareIdentity {
        StatusCode::UNAUTHORIZED
    } else {
        StatusCode::FORBIDDEN
    };
    let key = format!("gateway-{}", kind.replace('_', "-"));
    pages::error(
        status,
        kind,
        &crate::l10n::text(langs, &key, &[]),
        langs,
        format,
    )
}

pub(super) async fn dispatch(
    gateway: Arc<Gateway>,
    core: axum::Router,
    req: axum::extract::Request,
) -> Response {
    request::dispatch(gateway, core, req).await
}

#[cfg(test)]
mod tests;
