use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use wonton_protocol::ShareGrant;

use crate::friends::FriendRecord;

pub const REVOCATION_RETENTION_MS: u64 = 24 * 60 * 60 * 1000;
pub const MAX_REVOCATIONS: usize = 4096;

/// These facts must come from the installed plugin registry, never from an API request body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginOffer {
    pub plugin_id: String,
    pub installed: bool,
    pub has_webui: bool,
    pub per_user_accounts: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Access {
    pub friend: FriendRecord,
    /// Most permissive union: None if any effective grant is unbounded; otherwise latest expiry.
    pub expires_at_ms: Option<u64>,
    /// Qualifying current grants. A connection survives only while at least one original permit
    /// remains in this set, in addition to its friendship-generation and expiry checks.
    pub permits: Vec<GrantPermit>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrantPermit {
    pub grant_id: String,
    pub generation: u64,
    pub group_generation: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatewayRefusal {
    ShareRequired,
    ShareRevoked,
    ShareExpired,
}

#[derive(Debug, thiserror::Error)]
pub enum SharesError {
    #[error("share registry I/O failed at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("share registry is corrupt at {path}: {message}")]
    Corrupt { path: PathBuf, message: String },
    #[error("invalid share grant: {0}")]
    Invalid(String),
    #[error("share grant not found: {0}")]
    NotFound(String),
    #[error("plugin is not installed with a web UI: {0}")]
    PluginUnavailable(String),
    #[error("sharing this plugin requires explicit risk acknowledgement")]
    RiskAcknowledgementRequired,
    #[error("share registry revision exhausted")]
    CounterExhausted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredGrant {
    pub grant: ShareGrant,
    /// Committing shares revision on creation or complete replacement, even identical updates.
    pub generation: u64,
    /// Friends registry revision at create/complete replacement. Reaccepted friends with a newer
    /// activation cannot inherit this group grant; first-time friends may still join dynamically.
    pub friend_revision: u64,
    /// Required exactly for direct grants; compared against the active friendship generation.
    pub direct_generation: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Revocation {
    pub user_id: String,
    pub plugin_id: String,
    pub at_ms: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub revision: u64,
    pub grants: Vec<StoredGrant>,
    /// Diagnostic only: bounded historical facts must never grant access.
    pub revocations: Vec<Revocation>,
}

impl Snapshot {
    pub(super) fn remember(&mut self, user_id: &str, plugin_id: &str, now_ms: u64) {
        self.revocations
            .retain(|r| !(r.user_id == user_id && r.plugin_id == plugin_id));
        self.revocations.push(Revocation {
            user_id: user_id.to_owned(),
            plugin_id: plugin_id.to_owned(),
            at_ms: now_ms,
        });
        self.prune(now_ms);
    }

    pub(super) fn prune(&mut self, now_ms: u64) {
        self.revocations
            .retain(|r| now_ms < r.at_ms.saturating_add(REVOCATION_RETENTION_MS));
        self.revocations.sort_by(|a, b| {
            (a.at_ms, &a.user_id, &a.plugin_id).cmp(&(b.at_ms, &b.user_id, &b.plugin_id))
        });
        if self.revocations.len() > MAX_REVOCATIONS {
            self.revocations
                .drain(..self.revocations.len() - MAX_REVOCATIONS);
        }
    }
}
