use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use tokio::sync::watch;
use wonton_protocol::{PluginShares, ShareGrant, ShareGrantUpdate, ShareRecipient};

use crate::friends::{Friends, Snapshot as FriendsSnapshot};

use super::evaluation::{self, generation_matches, recipient_matches};
use super::{Access, GatewayRefusal, PluginOffer, SharesError, Snapshot, StoredGrant, persistence};

type Clock = Arc<dyn Fn() -> u64 + Send + Sync>;

pub struct Shares {
    path: PathBuf,
    friends: Arc<Friends>,
    writer: Mutex<()>,
    snapshots: watch::Sender<Arc<Snapshot>>,
    clock: Clock,
}

impl Shares {
    pub fn open(path: impl Into<PathBuf>, friends: Arc<Friends>) -> Result<Self, SharesError> {
        Self::with_clock(path, friends, Arc::new(system_ms))
    }

    pub fn with_clock(
        path: impl Into<PathBuf>,
        friends: Arc<Friends>,
        clock: Clock,
    ) -> Result<Self, SharesError> {
        let path = path.into();
        let snapshot = persistence::read(&path)?;
        let (snapshots, _) = watch::channel(Arc::new(snapshot));
        Ok(Self {
            path,
            friends,
            writer: Mutex::new(()),
            snapshots,
            clock,
        })
    }

    pub fn snapshot(&self) -> Arc<Snapshot> {
        Arc::clone(&self.snapshots.borrow())
    }

    /// Also subscribe to Friends: group membership and activation changes do not rewrite shares.
    pub fn subscribe(&self) -> watch::Receiver<Arc<Snapshot>> {
        self.snapshots.subscribe()
    }

    pub fn get(&self, id: &str) -> Option<ShareGrant> {
        self.snapshots
            .borrow()
            .grants
            .iter()
            .find(|s| s.grant.id == id)
            .map(|s| s.grant.clone())
    }

    pub fn create(
        &self,
        update: &ShareGrantUpdate,
        offer: &PluginOffer,
    ) -> Result<ShareGrant, SharesError> {
        self.change(|snapshot, friends, now| {
            let id = format!("share-{}", next_revision(snapshot)?);
            let stored = bind(id, next_revision(snapshot)?, update, offer, friends, now)?;
            let grant = stored.grant.clone();
            snapshot.grants.push(stored);
            Ok(grant)
        })
    }

    /// Complete replacement, retaining only the ID; recipient/plugin/expiry and bindings are fresh.
    pub fn update(
        &self,
        id: &str,
        update: &ShareGrantUpdate,
        offer: &PluginOffer,
    ) -> Result<ShareGrant, SharesError> {
        self.change(|snapshot, friends, now| {
            let index = snapshot
                .grants
                .iter()
                .position(|s| s.grant.id == id)
                .ok_or_else(|| SharesError::NotFound(id.to_owned()))?;
            let replacement = bind(
                id.to_owned(),
                next_revision(snapshot)?,
                update,
                offer,
                friends,
                now,
            )?;
            let previous = snapshot.grants[index].clone();
            remember_recipients(snapshot, &previous, Some(&replacement), friends, now);
            let grant = replacement.grant.clone();
            snapshot.grants[index] = replacement;
            Ok(grant)
        })
    }

    pub fn revoke(&self, id: &str) -> Result<(), SharesError> {
        self.change(|snapshot, friends, now| {
            let index = snapshot
                .grants
                .iter()
                .position(|s| s.grant.id == id)
                .ok_or_else(|| SharesError::NotFound(id.to_owned()))?;
            let removed = snapshot.grants.remove(index);
            remember_recipients(snapshot, &removed, None, friends, now);
            Ok(())
        })
    }

    /// Persist before uninstall completes. Deleted grants cannot revive if the same plugin returns.
    pub fn revoke_plugin(&self, plugin_id: &str) -> Result<(), SharesError> {
        self.change(|snapshot, friends, now| {
            let removed: Vec<_> = snapshot
                .grants
                .iter()
                .filter(|s| s.grant.plugin_id == plugin_id)
                .cloned()
                .collect();
            snapshot.grants.retain(|s| s.grant.plugin_id != plugin_id);
            for stored in removed {
                remember_recipients(snapshot, &stored, None, friends, now);
            }
            Ok(())
        })
    }

    /// Current grant union only, not plugin health or transport authentication. The gateway must
    /// additionally compare its authenticated generation with Access.friend.access_generation.
    pub fn evaluate(
        &self,
        user_id: &str,
        plugin_id: &str,
        now_ms: u64,
    ) -> Result<Access, GatewayRefusal> {
        let _writer = self.writer.lock().expect("share registry lock poisoned");
        let shares = self.snapshot();
        let friends = self.friends.snapshot();
        evaluation::evaluate(&shares, &friends, user_id, plugin_id, now_ms)
    }

    pub fn active_plugin_ids(&self, user_id: &str, now_ms: u64) -> Vec<String> {
        let _writer = self.writer.lock().expect("share registry lock poisoned");
        let shares = self.snapshot();
        let friends = self.friends.snapshot();
        let ids: BTreeSet<_> = shares.grants.iter().map(|s| &s.grant.plugin_id).collect();
        ids.into_iter()
            .filter(|id| evaluation::evaluate(&shares, &friends, user_id, id, now_ms).is_ok())
            .cloned()
            .collect()
    }

    pub fn plugin_shares(
        &self,
        offer: &PluginOffer,
        now_ms: u64,
    ) -> Result<PluginShares, SharesError> {
        validate_offer(offer, &offer.plugin_id)?;
        let _writer = self.writer.lock().expect("share registry lock poisoned");
        let shares = self.snapshot();
        let friends = self.friends.snapshot();
        let count = friends
            .active()
            .filter(|friend| {
                evaluation::evaluate(&shares, &friends, &friend.user_id, &offer.plugin_id, now_ms)
                    .is_ok()
            })
            .count();
        Ok(PluginShares {
            friends_supported: offer.friends_supported,
            friend_count: u32::try_from(count).map_err(|_| {
                SharesError::Invalid("friend count exceeds protocol capacity".into())
            })?,
            grants: shares
                .grants
                .iter()
                .filter(|s| s.grant.plugin_id == offer.plugin_id)
                .map(|s| s.grant.clone())
                .collect(),
        })
    }

    /// Earliest future grant expiry. Re-arm after either registry changes or this deadline fires.
    /// Expired grants remain stored for diagnosis, but never authorize at or after the deadline.
    pub fn next_deadline(&self, now_ms: u64) -> Option<u64> {
        self.snapshots
            .borrow()
            .grants
            .iter()
            .filter_map(|s| s.grant.expires_at_ms)
            .filter(|at| *at > now_ms)
            .min()
    }

    fn change<T>(
        &self,
        change: impl FnOnce(&mut Snapshot, &FriendsSnapshot, u64) -> Result<T, SharesError>,
    ) -> Result<T, SharesError> {
        let _writer = self.writer.lock().expect("share registry lock poisoned");
        let before = self.snapshot();
        let friends = self.friends.snapshot();
        let now = (self.clock)();
        let mut next = before.as_ref().clone();
        let result = change(&mut next, &friends, now)?;
        if next != *before {
            next.prune(now);
            next.revision = next_revision(&before)?;
            persistence::write(&self.path, &next)?;
            self.snapshots.send_replace(Arc::new(next));
        }
        Ok(result)
    }
}

fn validate_offer(offer: &PluginOffer, plugin_id: &str) -> Result<(), SharesError> {
    if offer.plugin_id != plugin_id || plugin_id.is_empty() || !offer.installed || !offer.has_webui
    {
        return Err(SharesError::PluginUnavailable(plugin_id.to_owned()));
    }
    Ok(())
}

fn bind(
    id: String,
    generation: u64,
    update: &ShareGrantUpdate,
    offer: &PluginOffer,
    friends: &FriendsSnapshot,
    now_ms: u64,
) -> Result<StoredGrant, SharesError> {
    validate_offer(offer, &update.plugin_id)?;
    if !offer.friends_supported && !update.acknowledge_risk {
        return Err(SharesError::RiskAcknowledgementRequired);
    }
    if update.expires_at_ms.is_some_and(|at| at <= now_ms) {
        return Err(SharesError::Invalid(
            "grant expiry must be in the future".into(),
        ));
    }
    let direct_generation = match &update.recipient {
        ShareRecipient::Friend { user_id } => Some(
            friends
                .get(user_id)
                .ok_or_else(|| SharesError::Invalid("recipient is not an active friend".into()))?
                .access_generation,
        ),
        ShareRecipient::Group { group_id } => {
            if !friends.groups.iter().any(|group| group.id == *group_id) {
                return Err(SharesError::Invalid(
                    "recipient group does not exist".into(),
                ));
            }
            None
        }
        _ => return Err(SharesError::Invalid("unknown share recipient".into())),
    };
    Ok(StoredGrant {
        grant: ShareGrant {
            id,
            plugin_id: update.plugin_id.clone(),
            recipient: update.recipient.clone(),
            expires_at_ms: update.expires_at_ms,
        },
        generation,
        friend_revision: friends.revision,
        direct_generation,
    })
}

fn remember_recipients(
    snapshot: &mut Snapshot,
    stored: &StoredGrant,
    replacement: Option<&StoredGrant>,
    friends: &FriendsSnapshot,
    now_ms: u64,
) {
    let retained = |user_id: &str| {
        replacement.is_some_and(|replacement| {
            stored.grant.plugin_id == replacement.grant.plugin_id
                && friends.get(user_id).is_some_and(|friend| {
                    recipient_matches(replacement, friend)
                        && generation_matches(replacement, friend)
                })
        })
    };
    if let ShareRecipient::Friend { user_id } = &stored.grant.recipient {
        if !retained(user_id) {
            snapshot.remember(user_id, &stored.grant.plugin_id, now_ms);
        }
        return;
    }
    for friend in friends
        .active()
        .filter(|f| recipient_matches(stored, f) && generation_matches(stored, f))
    {
        if !retained(&friend.user_id) {
            snapshot.remember(&friend.user_id, &stored.grant.plugin_id, now_ms);
        }
    }
}

fn next_revision(snapshot: &Snapshot) -> Result<u64, SharesError> {
    snapshot
        .revision
        .checked_add(1)
        .ok_or(SharesError::CounterExhausted)
}

fn system_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            duration.as_millis().min(u128::from(u64::MAX)) as u64
        })
}
