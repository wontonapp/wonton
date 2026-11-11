use std::{
    collections::BTreeMap,
    sync::Mutex,
    time::{Duration, Instant},
};

use super::{CACHE_CAPACITY, DISCOVERY_TTL};

/// Monotonic clock seam for deterministic expiry; never derived from token expires_in.
pub trait DiscoveryClock: Send + Sync {
    fn now(&self) -> Duration;
}

pub(super) struct MonotonicClock(pub Instant);
impl DiscoveryClock for MonotonicClock {
    fn now(&self) -> Duration {
        self.0.elapsed()
    }
}

pub(super) struct Cache<T>(Mutex<BTreeMap<String, (Duration, T)>>);
impl<T: Clone> Cache<T> {
    pub(super) fn new() -> Self {
        Self(Mutex::new(BTreeMap::new()))
    }

    pub(super) fn get(&self, key: &str, now: Duration) -> Option<T> {
        let mut values = self.0.lock().ok()?;
        values.retain(|_, (created, _)| {
            now.checked_sub(*created)
                .is_some_and(|age| age < DISCOVERY_TTL)
        });
        values.get(key).map(|(_, value)| value.clone())
    }

    pub(super) fn insert(&self, key: String, value: T, now: Duration) {
        let Ok(mut values) = self.0.lock() else {
            return;
        };
        values.retain(|_, (created, _)| {
            now.checked_sub(*created)
                .is_some_and(|age| age < DISCOVERY_TTL)
        });
        if values.len() == CACHE_CAPACITY && !values.contains_key(&key) {
            let oldest = values
                .iter()
                .min_by_key(|(_, (created, _))| *created)
                .map(|(key, _)| key.clone());
            if let Some(oldest) = oldest {
                values.remove(&oldest);
            }
        }
        values.insert(key, (now, value));
    }

    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.0.lock().unwrap().len()
    }
}
