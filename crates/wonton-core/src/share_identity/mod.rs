//! Matrix OpenID identity verification for `wonton/share/0` (ADR-0051).
//!
//! Only discovery results are cached. Every connection performs userinfo; `expires_in` is
//! input validation, never authorization. All outward failures are existing protocol categories.
//! No underlying HTTP/DNS/JSON error or token-bearing URL is returned, formatted or logged.
//! Direct requests use validated pinned public addresses with no proxy; paired requests use
//! the existing iroh federation CONNECT forwarder with ordinary strict end-to-end TLS.

mod cache;
mod discovery;
mod dns;
mod https;
mod name;

pub use cache::DiscoveryClock;
pub use dns::{IdentityDns, IdentityFuture, SrvRecord};
pub use https::PairedLookup;
pub use name::MatrixServerName;

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use wonton_protocol::{MatrixOpenIdToken, ShareIdentityFailure, WontonDiscovery};

use cache::{Cache, MonotonicClock};
use dns::SystemDns;
use https::{Destination, Exchange, ExchangeRequest, HttpsExchange, resolve_public};

pub const TOTAL_TIMEOUT: Duration = Duration::from_secs(10);
pub const MAX_RESPONSE_BYTES: usize = 64 * 1024;
pub const DISCOVERY_TTL: Duration = Duration::from_secs(3600);
pub const CACHE_CAPACITY: usize = 256;
pub const MAX_ADDRESSES: usize = 32;
pub const MAX_SRV_RECORDS: usize = 32;
pub const MAX_REDIRECTS: usize = 5;
pub const MAX_TOKEN_BYTES: usize = 16 * 1024;

/// Stateful discovery client. Share behind Arc; it contains no token or authenticated-user cache.
/// No Debug implementation: injected collaborators must not accidentally expose credentials.
pub struct IdentityVerifier {
    dns: Arc<dyn IdentityDns>,
    http: Arc<dyn Exchange>,
    paired: PairedLookup,
    clock: Arc<dyn DiscoveryClock>,
    matrix_cache: Cache<Vec<Destination>>,
    wonton_cache: Cache<WontonDiscovery>,
}

impl IdentityVerifier {
    /// Construct with the existing core peer-table/forwarder binding. Return None only when
    /// the server is not paired; never return None because a paired server is offline.
    /// The current forwarder supports only 8448; paired names with another explicit port
    /// return DiscoveryFailed without public DNS fallback. Its inbound must serve a valid
    /// certificate for the Matrix server name (or a CA explicitly added with trusted roots).
    pub fn new(paired: PairedLookup) -> Result<Self, ShareIdentityFailure> {
        Ok(Self::with_dns(
            Arc::new(SystemDns::new()?),
            paired,
            Arc::new(MonotonicClock(Instant::now())),
        ))
    }

    /// Explicit DNS and monotonic clock injection. HTTPS policy cannot be replaced by callers.
    pub fn with_dns(
        dns: Arc<dyn IdentityDns>,
        paired: PairedLookup,
        clock: Arc<dyn DiscoveryClock>,
    ) -> Self {
        Self {
            dns,
            paired,
            clock,
            http: Arc::new(HttpsExchange { roots: Vec::new() }),
            matrix_cache: Cache::new(),
            wonton_cache: Cache::new(),
        }
    }

    /// Add an explicitly trusted CA (e.g. a deployment's private CA or a local test CA).
    /// This does not disable chain, expiry or hostname verification.
    pub fn with_trusted_roots(mut self, roots: Vec<reqwest::Certificate>) -> Self {
        self.http = Arc::new(HttpsExchange { roots });
        self
    }

    /// Returns the homeserver-verified Matrix user ID. No sharing authorization is implied.
    pub async fn verify(&self, token: &MatrixOpenIdToken) -> Result<String, ShareIdentityFailure> {
        if token.access_token.expose().is_empty()
            || token.access_token.expose().len() > MAX_TOKEN_BYTES
            || token.expires_in == 0
        {
            return Err(ShareIdentityFailure::InvalidToken);
        }
        let name = MatrixServerName::parse(&token.matrix_server_name)?;
        self.verify_inner(token, &name, tokio::time::Instant::now() + TOTAL_TIMEOUT)
            .await
    }

    async fn verify_inner(
        &self,
        token: &MatrixOpenIdToken,
        name: &MatrixServerName,
        deadline: tokio::time::Instant,
    ) -> Result<String, ShareIdentityFailure> {
        if let Some(forwarder) = (self.paired)(name) {
            // Existing forwarder accepts only the Matrix federation port. No public discovery
            // or DNS occurs in this branch; CONNECT is authenticated via its peer lookup.
            if name.explicit_port.is_some_and(|port| port != 8448) {
                return Err(ShareIdentityFailure::DiscoveryFailed);
            }
            return tokio::time::timeout_at(
                deadline,
                self.userinfo(
                    token,
                    name,
                    &Destination::direct(name.clone(), 8448),
                    Some(forwarder),
                ),
            )
            .await
            .unwrap_or(Err(ShareIdentityFailure::UserInfoFailed));
        }
        let destinations = tokio::time::timeout_at(deadline, self.discover(name))
            .await
            .unwrap_or(Err(ShareIdentityFailure::DiscoveryFailed))?;
        let mut failure = ShareIdentityFailure::UserInfoFailed;
        // Finite SRV alternatives only, no retries or fallback after a definitive userinfo reply.
        for destination in destinations {
            match tokio::time::timeout_at(deadline, self.userinfo(token, name, &destination, None))
                .await
                .unwrap_or(Err(ShareIdentityFailure::UserInfoFailed))
            {
                Err(ShareIdentityFailure::DiscoveryFailed) => {
                    failure = ShareIdentityFailure::DiscoveryFailed
                }
                result => return result,
            }
        }
        Err(failure)
    }

    async fn userinfo(
        &self,
        token: &MatrixOpenIdToken,
        identity: &MatrixServerName,
        destination: &Destination,
        paired: Option<Arc<wonton_tunnel::federation::FederationForwarder>>,
    ) -> Result<String, ShareIdentityFailure> {
        let addresses = if paired.is_some() {
            Vec::new()
        } else {
            resolve_public(self.dns.as_ref(), &destination.target, destination.port).await?
        };
        let mut url = destination.url("/_matrix/federation/v1/openid/userinfo")?;
        // The Matrix federation specification requires this query parameter. Never follow
        // userinfo redirects: a redirect must not carry the bearer token to another origin.
        url.query_pairs_mut()
            .append_pair("access_token", token.access_token.expose());
        let response = self
            .http
            .get(ExchangeRequest {
                url,
                host_header: Some(destination.authority.authority()),
                addresses,
                paired,
            })
            .await?;
        if matches!(response.status.as_u16(), 401 | 403) {
            return Err(ShareIdentityFailure::InvalidToken);
        }
        if !response.status.is_success() {
            return Err(ShareIdentityFailure::UserInfoFailed);
        }
        #[derive(serde::Deserialize)]
        struct UserInfo {
            sub: String,
        }
        let info: UserInfo = serde_json::from_slice(&response.body)
            .map_err(|_| ShareIdentityFailure::UserInfoFailed)?;
        checked_subject(&info.sub, identity)
    }
}

fn checked_subject(
    subject: &str,
    identity: &MatrixServerName,
) -> Result<String, ShareIdentityFailure> {
    let user =
        ruma_common::UserId::parse(subject).map_err(|_| ShareIdentityFailure::UserInfoFailed)?;
    let server = MatrixServerName::parse(user.server_name().as_str())
        .map_err(|_| ShareIdentityFailure::UserInfoFailed)?;
    if !server.same_identity(identity) {
        return Err(ShareIdentityFailure::ServerNameMismatch);
    }
    Ok(subject.to_owned())
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod tests_tls;
