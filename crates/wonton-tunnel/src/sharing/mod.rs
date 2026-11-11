//! Connection-scoped Matrix OpenID sharing (ADR-0051).
//!
//! Register [`ShareProtocol`] on [`SHARE_ALPN`]. Only the authentication handler may
//! establish the Matrix identity; endpoint keys are transport identities, not users.
//! Core must rate-limit connections before dispatch, validate every HTTP request and
//! actively revoke its forwarding streams when grants change. No token is retained
//! by the client or server after authentication, and no reconnect is attempted.
//!
//! Wire: every bidirectional stream starts with Forward=0 or Control=1. A control
//! stream contains one big-endian u32 length followed by JSON and FIN in each
//! direction (8 KiB request / 1 MiB reply). Forward streams carry raw HTTP, without
//! an additional status byte. Authentication, including its reply, has a total
//! ten-second deadline. Any second stream arriving before authentication completes
//! closes the connection; it is never queued for later authenticated forwarding.

mod client;
mod frame;
mod proxy;
mod proxy_hub;
mod server;
mod stream;

pub use client::ShareClient;
pub use proxy::ShareLocalProxy;
pub use proxy_hub::{ProxyHub, ProxyRegistration};
pub use server::ShareProtocol;
pub use stream::ShareStream;
pub use wonton_protocol::SHARE_ALPN;

use std::{future::Future, pin::Pin};

use iroh::endpoint::Connection;
use wonton_protocol::{GatewayRefusal, MatrixOpenIdToken, ShareIdentityFailure, SharedPlugins};

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// The result contains only protocol enums, never token-bearing upstream errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShareRefusal {
    pub refusal: GatewayRefusal,
    pub identity: Option<ShareIdentityFailure>,
}

impl ShareRefusal {
    pub fn identity(reason: ShareIdentityFailure) -> Self {
        Self {
            refusal: GatewayRefusal::ShareIdentity,
            identity: Some(reason),
        }
    }
}

/// Core's authorization epoch is private transport state, not a wire field.
#[derive(Clone, Debug)]
pub struct AuthenticatedShares {
    pub shares: SharedPlugins,
    pub generation: u64,
}

#[derive(Clone, Debug)]
pub struct ShareSession {
    user_id: std::sync::Arc<str>,
    pub generation: u64,
}

impl ShareSession {
    pub fn user_id(&self) -> &str {
        &self.user_id
    }
}

/// The caller must use the verified userinfo result in `SharedPlugins.user_id`,
/// never a token field, request parameter or endpoint key. Implementations must not
/// record the token. `forward` lives until the HTTP service finishes; core may also
/// retain a connection clone to close all streams belonging to that session.
pub trait ShareHandler: Send + Sync + 'static {
    fn authenticate(
        &self,
        token: MatrixOpenIdToken,
    ) -> BoxFuture<'_, Result<AuthenticatedShares, ShareRefusal>>;
    fn list<'a>(
        &'a self,
        session: &'a ShareSession,
    ) -> BoxFuture<'a, Result<SharedPlugins, ShareRefusal>>;
    fn forward<'a>(
        &'a self,
        session: &'a ShareSession,
        connection: Connection,
        stream: ShareStream,
    ) -> BoxFuture<'a, ()>;
}

#[derive(Clone, Copy, Debug)]
pub struct ShareLimits {
    /// Includes controls, forwarding tasks and partial stream headers.
    pub max_active_streams: u32,
}

impl Default for ShareLimits {
    fn default() -> Self {
        Self {
            max_active_streams: 64,
        }
    }
}

/// Sanitized public errors: network and JSON diagnostics never echo credentials.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ShareError {
    #[error("share transport unavailable")]
    Transport,
    #[error("share request timed out")]
    Timeout,
    #[error("invalid share protocol message")]
    Protocol,
    #[error("share message exceeds size limit")]
    TooLarge,
    #[error("share request refused: {0:?}")]
    Refused(ShareRefusal),
    #[error("share connection closed")]
    Closed,
}

#[cfg(test)]
mod tests;
