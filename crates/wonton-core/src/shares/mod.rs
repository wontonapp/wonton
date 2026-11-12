//! Person-based plugin grants (ADR-0051). `shares.json` travels with the existing core backup.
//! PluginOffer facts are trusted server observations, never client-supplied authority. Callers
//! subscribe to both Shares and Friends before checking access; either registry changes access.
//! Bind each connection to the authenticated friend's access_generation and recheck it on every
//! request and invalidation. Expiry timers must use next_deadline or the returned union expiry.
//! Preserve original grant permits and cancel a connection if none survives in current Access;
//! new/replaced grants and removed/readded memberships cannot revive an existing connection.
//! Rename commits a write; unsupported/failed post-rename directory sync warns about crash durability.

mod evaluation;
mod persistence;
mod store;
mod types;

pub use store::Shares;
pub use types::{
    Access, GatewayRefusal, GrantPermit, PluginOffer, SharesError, Snapshot, StoredGrant,
};

#[cfg(test)]
mod tests;
