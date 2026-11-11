use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use tokio::sync::watch;
use wonton_protocol::FriendGroup;

use super::persistence;
use super::types::{group_name, validate_display_name, validate_groups, validate_user_id};
use super::{FriendRecord, FriendStatus, FriendsError, Snapshot};

/// One registry instance per file, shared with `Arc`. Mutations are serialized through persistence
/// and publication; consumers never observe speculative state or out-of-order notifications.
#[derive(Debug)]
pub struct Friends {
    path: PathBuf,
    writer: Mutex<()>,
    snapshots: watch::Sender<Arc<Snapshot>>,
}

impl Friends {
    /// Open `core_dir.join("friends.json")`. Missing means empty; malformed data is never replaced.
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, FriendsError> {
        let path = path.into();
        let snapshot = persistence::read(&path)?;
        let (snapshots, _) = watch::channel(Arc::new(snapshot));
        Ok(Self {
            path,
            writer: Mutex::new(()),
            snapshots,
        })
    }

    /// Subscribe before the first authorization check; use `borrow_and_update()` then `changed()`.
    /// Every update carries the complete registry, even if several revisions were coalesced.
    pub fn subscribe(&self) -> watch::Receiver<Arc<Snapshot>> {
        self.snapshots.subscribe()
    }

    pub fn snapshot(&self) -> Arc<Snapshot> {
        Arc::clone(&self.snapshots.borrow())
    }

    /// Active lookup only. Removed identities are accessible through `list_including_removed()`.
    pub fn get(&self, user_id: &str) -> Option<FriendRecord> {
        self.snapshots.borrow().get(user_id).cloned()
    }

    pub fn find_username(&self, username: &str) -> Option<FriendRecord> {
        self.snapshots.borrow().find_username(username).cloned()
    }

    pub fn list_active(&self) -> Vec<FriendRecord> {
        self.snapshots.borrow().active().cloned().collect()
    }

    pub fn list_including_removed(&self) -> Vec<FriendRecord> {
        self.snapshots.borrow().friends.clone()
    }

    /// Includes tombstones. Local account creation must also reserve these plugin usernames.
    pub fn taken(&self, username: &str) -> bool {
        self.snapshots
            .borrow()
            .friends
            .iter()
            .any(|f| f.username == username)
    }

    /// Call only after Matrix invitation acceptance. The required callback returns true for any
    /// local owner/member (including removed)/bot/existing Matrix username. Prepare those facts
    /// before calling, keep their namespace reservation serialized with local account creation,
    /// and never reenter this store from the callback. No dependency on display-name uniqueness.
    /// Reacceptance keeps this identity's username and first acceptance, but never restores groups.
    pub fn accept(
        &self,
        user_id: &str,
        display_name: Option<&str>,
        accepted_at_ms: u64,
        local_name_taken: impl Fn(&str) -> bool,
    ) -> Result<FriendRecord, FriendsError> {
        self.accept_candidate(
            user_id,
            display_name,
            accepted_at_ms,
            local_name_taken,
            random_username,
        )
    }

    /// Prepare a username for async local-account checks. Hold the shared person-name lock
    /// through `accept_candidate`, passing this same name after checking Matrix.
    pub fn candidate_username(&self, user_id: &str) -> Result<String, FriendsError> {
        validate_user_id(user_id)?;
        if let Some(friend) = self
            .snapshot()
            .friends
            .iter()
            .find(|friend| friend.user_id == user_id)
        {
            return Ok(friend.username.clone());
        }
        random_username()
    }

    pub fn accept_candidate(
        &self,
        user_id: &str,
        display_name: Option<&str>,
        accepted_at_ms: u64,
        local_name_taken: impl Fn(&str) -> bool,
        candidate: impl FnOnce() -> Result<String, FriendsError>,
    ) -> Result<FriendRecord, FriendsError> {
        validate_user_id(user_id)?;
        validate_display_name(display_name)?;
        self.change(|snapshot| {
            let activation_revision = snapshot.revision.checked_add(1);
            if let Some(friend) = snapshot.friends.iter_mut().find(|f| f.user_id == user_id) {
                if local_name_taken(&friend.username) {
                    return Err(FriendsError::UsernameTaken(friend.username.clone()));
                }
                if friend.status == FriendStatus::Removed {
                    friend.access_generation =
                        activation_revision.ok_or(FriendsError::CounterExhausted)?;
                    friend.groups.clear();
                    friend.group_generations.clear();
                }
                friend.status = FriendStatus::Active;
                friend.removed_at_ms = None;
                friend.display_name = display_name.map(str::to_owned);
                return Ok(friend.clone());
            }
            let username = candidate()?;
            crate::owner::check_username(&username).map_err(FriendsError::Invalid)?;
            crate::owner::check_not_reserved(&username).map_err(FriendsError::Invalid)?;
            if snapshot.friends.iter().any(|f| f.username == username)
                || local_name_taken(&username)
            {
                return Err(FriendsError::UsernameTaken(username));
            }
            let friend = FriendRecord {
                user_id: user_id.to_owned(),
                display_name: display_name.map(str::to_owned),
                username,
                status: FriendStatus::Active,
                access_generation: next_revision(snapshot)?,
                groups: Vec::new(),
                group_generations: Default::default(),
                accepted_order: next_revision(snapshot)?,
                accepted_at_ms,
                removed_at_ms: None,
                last_opened_share_at_ms: None,
            };
            snapshot.friends.push(friend.clone());
            Ok(friend)
        })
    }

    /// Retain the username tombstone and revoke all group memberships. Repeated removal is a no-op.
    pub fn remove(&self, user_id: &str, removed_at_ms: u64) -> Result<FriendRecord, FriendsError> {
        self.change(|snapshot| {
            let friend = snapshot
                .friends
                .iter_mut()
                .find(|f| f.user_id == user_id)
                .ok_or_else(|| FriendsError::NotFound(user_id.to_owned()))?;
            friend.status = FriendStatus::Removed;
            friend.removed_at_ms = Some(removed_at_ms);
            friend.groups.clear();
            friend.group_generations.clear();
            Ok(friend.clone())
        })
    }

    pub fn update_display_name(
        &self,
        user_id: &str,
        display_name: Option<&str>,
    ) -> Result<FriendRecord, FriendsError> {
        validate_display_name(display_name)?;
        self.update(user_id, |friend| {
            friend.display_name = display_name.map(str::to_owned)
        })
    }

    /// Create when `id` is absent; rename an existing group otherwise. IDs never change or get reused.
    pub fn save_group(&self, id: Option<&str>, name: &str) -> Result<FriendGroup, FriendsError> {
        let name = group_name(name)?;
        self.change(|snapshot| {
            if let Some(id) = id {
                let group = snapshot
                    .groups
                    .iter_mut()
                    .find(|group| group.id == id)
                    .ok_or_else(|| FriendsError::GroupNotFound(id.to_owned()))?;
                group.name = name;
                return Ok(group.clone());
            }
            let group = FriendGroup {
                id: format!("group-{}", next_revision(snapshot)?),
                name,
            };
            snapshot.groups.push(group.clone());
            Ok(group)
        })
    }

    /// Delete a group and strip it from every friend in the same durable revision.
    pub fn remove_group(&self, id: &str) -> Result<(), FriendsError> {
        self.change(|snapshot| {
            let index = snapshot
                .groups
                .iter()
                .position(|group| group.id == id)
                .ok_or_else(|| FriendsError::GroupNotFound(id.to_owned()))?;
            snapshot.groups.remove(index);
            for friend in &mut snapshot.friends {
                friend.groups.retain(|group| group != id);
                friend.group_generations.remove(id);
            }
            Ok(())
        })
    }

    /// Full replacement. Duplicate/unknown group IDs reject the whole change.
    pub fn set_groups(
        &self,
        user_id: &str,
        groups: &[String],
    ) -> Result<FriendRecord, FriendsError> {
        self.change(|snapshot| {
            validate_groups(groups, &snapshot.groups)?;
            let membership_revision = snapshot.revision.checked_add(1);
            let friend = active_mut(snapshot, user_id)?;
            friend.group_generations.retain(|id, _| groups.contains(id));
            for id in groups {
                if !friend.group_generations.contains_key(id) {
                    friend.group_generations.insert(
                        id.clone(),
                        membership_revision.ok_or(FriendsError::CounterExhausted)?,
                    );
                }
            }
            friend.groups = groups.to_vec();
            friend.groups.sort();
            Ok(friend.clone())
        })
    }

    /// Gateway-only observation: call after actual accepted plugin access, never on listing shares
    /// or merely authenticating a transport. Stale observations cannot regress the recorded time.
    pub fn record_opened(&self, user_id: &str, at_ms: u64) -> Result<FriendRecord, FriendsError> {
        self.update(user_id, |friend| {
            friend.last_opened_share_at_ms = Some(
                friend
                    .last_opened_share_at_ms
                    .map_or(at_ms, |old| old.max(at_ms)),
            );
        })
    }

    fn update(
        &self,
        user_id: &str,
        change: impl FnOnce(&mut FriendRecord),
    ) -> Result<FriendRecord, FriendsError> {
        self.change(|snapshot| {
            let friend = active_mut(snapshot, user_id)?;
            change(friend);
            Ok(friend.clone())
        })
    }

    fn change<T>(
        &self,
        change: impl FnOnce(&mut Snapshot) -> Result<T, FriendsError>,
    ) -> Result<T, FriendsError> {
        let _writer = self.writer.lock().expect("friend registry lock poisoned");
        let before = self.snapshot();
        let mut next = before.as_ref().clone();
        let result = change(&mut next)?;
        if next != *before {
            next.revision = next_revision(&before)?;
            persistence::write(&self.path, &next)?;
            self.snapshots.send_replace(Arc::new(next));
        }
        Ok(result)
    }
}

fn active_mut<'a>(
    snapshot: &'a mut Snapshot,
    user_id: &str,
) -> Result<&'a mut FriendRecord, FriendsError> {
    snapshot
        .friends
        .iter_mut()
        .find(|f| f.user_id == user_id && f.status == FriendStatus::Active)
        .ok_or_else(|| FriendsError::NotFound(user_id.to_owned()))
}

fn next_revision(snapshot: &Snapshot) -> Result<u64, FriendsError> {
    snapshot
        .revision
        .checked_add(1)
        .ok_or(FriendsError::CounterExhausted)
}

fn random_username() -> Result<String, FriendsError> {
    let mut bytes = [0u8; 12];
    getrandom::fill(&mut bytes)
        .map_err(|error| FriendsError::Invalid(format!("OS randomness unavailable: {error}")))?;
    let mut username = String::with_capacity(31);
    username.push_str("friend-");
    use std::fmt::Write;
    for byte in bytes {
        write!(&mut username, "{byte:02x}").expect("writing to String cannot fail");
    }
    Ok(username)
}
