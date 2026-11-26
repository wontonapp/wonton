use std::collections::BTreeSet;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// 旧记录没有角色时算“朋友”：权限最少，不冒充家人。
fn default_role_id() -> String {
    crate::roles::DEFAULT_INVITE_ROLE.to_owned()
}

#[derive(Debug, thiserror::Error)]
pub enum FriendsError {
    #[error("friend registry I/O failed at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("friend registry is corrupt at {path}: {message}")]
    Corrupt { path: PathBuf, message: String },
    #[error("invalid friend registry input: {0}")]
    Invalid(String),
    #[error("active friend not found: {0}")]
    NotFound(String),
    #[error("plugin username already belongs to another local identity: {0}")]
    UsernameTaken(String),
    #[error("friend registry counter exhausted")]
    CounterExhausted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FriendStatus {
    Active,
    Removed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FriendRecord {
    /// Complete Matrix identity, never a localpart or a display name.
    pub user_id: String,
    /// Actual Matrix profile, absent if unavailable; never manufactured from an ID.
    pub display_name: Option<String>,
    /// Fixed local plugin identity, reserved even when this friend is removed.
    pub username: String,
    pub status: FriendStatus,
    /// 好友名单里这个人的角色（`roles.json` 的 ID）；旧记录没有时算“朋友”。
    #[serde(default = "default_role_id")]
    pub role_id: String,
    /// Revision of the latest activation. Grants and connections must bind this generation;
    /// removing and reaccepting the same person must not revive their previous authorization.
    pub access_generation: u64,
    /// First acceptance order and timestamp; reacceptance preserves both.
    pub accepted_order: u64,
    pub accepted_at_ms: u64,
    /// 删除的时刻；重新接受后清空。好友页靠它区分“删掉前就在的房间”和删除后的新邀请。
    #[serde(default)]
    pub removed_at_ms: Option<u64>,
    pub last_opened_share_at_ms: Option<u64>,
}

/// Complete durable registry. `friends` includes tombstones; use `active()`/`get()` for access.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub revision: u64,
    pub friends: Vec<FriendRecord>,
}

impl Snapshot {
    /// Active friends in first-acceptance order.
    pub fn active(&self) -> impl Iterator<Item = &FriendRecord> {
        self.friends
            .iter()
            .filter(|f| f.status == FriendStatus::Active)
    }

    pub fn get(&self, user_id: &str) -> Option<&FriendRecord> {
        self.active().find(|f| f.user_id == user_id)
    }

    pub fn find_username(&self, username: &str) -> Option<&FriendRecord> {
        self.active().find(|f| f.username == username)
    }

    pub(super) fn validate(&self) -> Result<(), FriendsError> {
        let mut identities = BTreeSet::new();
        let mut usernames = BTreeSet::new();
        let mut previous_order = 0;
        for friend in &self.friends {
            validate_user_id(&friend.user_id)?;
            validate_display_name(friend.display_name.as_deref())?;
            crate::owner::check_username(&friend.username).map_err(FriendsError::Invalid)?;
            if !identities.insert(&friend.user_id) || !usernames.insert(&friend.username) {
                return Err(FriendsError::Invalid(
                    "duplicate identity or plugin username".into(),
                ));
            }
            if friend.accepted_order <= previous_order || friend.accepted_order > self.revision {
                return Err(FriendsError::Invalid("invalid acceptance ordering".into()));
            }
            if friend.access_generation < friend.accepted_order
                || friend.access_generation > self.revision
            {
                return Err(FriendsError::Invalid(
                    "invalid friendship access generation".into(),
                ));
            }
            previous_order = friend.accepted_order;
        }
        Ok(())
    }
}

pub(super) fn validate_user_id(user_id: &str) -> Result<(), FriendsError> {
    ruma_common::UserId::parse(user_id)
        .map(|_| ())
        .map_err(|error| FriendsError::Invalid(format!("invalid Matrix user ID: {error}")))
}

pub(super) fn validate_display_name(name: Option<&str>) -> Result<(), FriendsError> {
    if let Some(name) = name {
        wonton_protocol::check_display_name(name)
            .map_err(|error| FriendsError::Invalid(error.to_string()))?;
    }
    Ok(())
}
