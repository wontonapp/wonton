use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use tokio::sync::watch;

use super::persistence;
use super::types::{validate_display_name, validate_user_id};
use super::{FriendRecord, FriendStatus, FriendsError, Snapshot};

/// One registry instance per file, shared with `Arc`. Mutations are serialized through persistence
/// and publication; consumers never observe speculative state or out-of-order notifications.
#[derive(Debug)]
pub struct Friends {
    path: PathBuf,
    writer: Mutex<()>,
    snapshots: watch::Sender<Arc<Snapshot>>,
    /// 最近一次“打开过”的时间，比快照里的新：只在内存里推进，写盘按 [`OPENED_WRITE_INTERVAL_MS`] 粗化。
    opened: Mutex<BTreeMap<String, u64>>,
    /// 已经落盘的打开时间，写盘间隔按它算。
    written: Mutex<BTreeMap<String, u64>>,
}

/// 同一好友两次写“最近打开过”的最小间隔。打开是每个资源的请求都算一次的流水账，逐次 fsync 太贵，
/// 而且不该让守着授权快照的连接跟着醒。
const OPENED_WRITE_INTERVAL_MS: u64 = 5 * 60 * 1000;

impl Friends {
    /// Open `core_dir.join("friends.json")`. Missing means empty; malformed data is never replaced.
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, FriendsError> {
        let path = path.into();
        let snapshot = persistence::read(&path)?;
        let opened: BTreeMap<String, u64> = snapshot
            .friends
            .iter()
            .filter_map(|f| Some((f.user_id.clone(), f.last_opened_share_at_ms?)))
            .collect();
        let (snapshots, _) = watch::channel(Arc::new(snapshot));
        Ok(Self {
            path,
            writer: Mutex::new(()),
            snapshots,
            opened: Mutex::new(opened.clone()),
            written: Mutex::new(opened),
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
        self.snapshots
            .borrow()
            .get(user_id)
            .cloned()
            .map(|friend| self.with_opened(friend))
    }

    pub fn find_username(&self, username: &str) -> Option<FriendRecord> {
        self.snapshots
            .borrow()
            .find_username(username)
            .cloned()
            .map(|friend| self.with_opened(friend))
    }

    pub fn list_active(&self) -> Vec<FriendRecord> {
        let opened = self.opened.lock().unwrap().clone();
        self.snapshots
            .borrow()
            .active()
            .cloned()
            .map(|mut friend| {
                overlay(&mut friend, &opened);
                friend
            })
            .collect()
    }

    pub fn list_including_removed(&self) -> Vec<FriendRecord> {
        let opened = self.opened.lock().unwrap().clone();
        self.snapshots
            .borrow()
            .friends
            .iter()
            .cloned()
            .map(|mut friend| {
                overlay(&mut friend, &opened);
                friend
            })
            .collect()
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
    /// Reacceptance keeps this identity's username and first acceptance.
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
                role_id: crate::roles::DEFAULT_INVITE_ROLE.to_owned(),
                access_generation: next_revision(snapshot)?,
                accepted_order: next_revision(snapshot)?,
                accepted_at_ms,
                removed_at_ms: None,
                last_opened_share_at_ms: None,
            };
            snapshot.friends.push(friend.clone());
            Ok(friend)
        })
    }

    /// Retain the username tombstone. Repeated removal is a no-op.
    pub fn remove(&self, user_id: &str, removed_at_ms: u64) -> Result<FriendRecord, FriendsError> {
        self.change(|snapshot| {
            let friend = snapshot
                .friends
                .iter_mut()
                .find(|f| f.user_id == user_id)
                .ok_or_else(|| FriendsError::NotFound(user_id.to_owned()))?;
            friend.status = FriendStatus::Removed;
            friend.removed_at_ms = Some(removed_at_ms);
            Ok(friend.clone())
        })
    }

    /// 好友名单里给他换一个角色（`roles.json` 的 ID）。角色在不在由调用方先查。
    pub fn set_role(&self, user_id: &str, role_id: &str) -> Result<FriendRecord, FriendsError> {
        self.update(user_id, |friend| friend.role_id = role_id.to_owned())
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

    /// Gateway-only observation: call after actual accepted plugin access, never on listing shares
    /// or merely authenticating a transport. Stale observations cannot regress the recorded time.
    pub fn record_opened(&self, user_id: &str, at_ms: u64) -> Result<FriendRecord, FriendsError> {
        let _writer = self.writer.lock().expect("friend registry lock poisoned");
        let mut friend = self
            .get(user_id)
            .ok_or_else(|| FriendsError::NotFound(user_id.into()))?;
        let mut opened = self.opened.lock().unwrap();
        let at_ms = at_ms.max(friend.last_opened_share_at_ms.unwrap_or_default());
        friend.last_opened_share_at_ms = Some(at_ms);
        let due = self
            .written
            .lock()
            .unwrap()
            .get(user_id)
            .is_none_or(|written| at_ms.saturating_sub(*written) >= OPENED_WRITE_INTERVAL_MS);
        opened.insert(user_id.to_owned(), at_ms);
        if !due {
            return Ok(friend);
        }
        let snapshot = self.snapshot();
        let mut next = snapshot.as_ref().clone();
        active_mut(&mut next, user_id)?.last_opened_share_at_ms = Some(at_ms);
        next.revision = next_revision(&snapshot)?;
        persistence::write(&self.path, &next)?;
        self.written
            .lock()
            .unwrap()
            .insert(user_id.to_owned(), at_ms);
        // 不 send_replace：授权守卫按快照的 access_generation 工作，打开时间不影响它们。
        Ok(friend)
    }

    /// 用内存里的最新打开时间盖过快照里的旧值。
    fn with_opened(&self, mut friend: FriendRecord) -> FriendRecord {
        if let Some(at) = self.opened.lock().unwrap().get(&friend.user_id) {
            friend.last_opened_share_at_ms =
                Some(friend.last_opened_share_at_ms.map_or(*at, |o| o.max(*at)));
        }
        friend
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
        // 比对和写盘都带上内存里的打开时间：真正的无操作不因为快照还落后一节就多写一次，
        // 真要写时也把这个不算通知的值一起落盘，不会被之后的无操作回退。
        let opened = self.opened.lock().unwrap().clone();
        let mut before = self.snapshot().as_ref().clone();
        for friend in &mut before.friends {
            overlay(friend, &opened);
        }
        let mut next = before.clone();
        let result = change(&mut next)?;
        if next != before {
            next.revision = next_revision(&before)?;
            persistence::write(&self.path, &next)?;
            self.snapshots.send_replace(Arc::new(next));
        }
        Ok(result)
    }
}

fn overlay(friend: &mut FriendRecord, opened: &BTreeMap<String, u64>) {
    if let Some(at) = opened.get(&friend.user_id) {
        friend.last_opened_share_at_ms = Some(
            friend
                .last_opened_share_at_ms
                .map_or(*at, |old| old.max(*at)),
        );
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
