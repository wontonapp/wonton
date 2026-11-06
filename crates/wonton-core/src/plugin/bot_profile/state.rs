use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use tokio::sync::Mutex as AsyncMutex;

use super::ProfileError;
use crate::plugin::PluginId;

const WINDOW: Duration = Duration::from_secs(3600);
const MAX_SUCCESSES: usize = 10;

/// 核心改过的资料版本。活动接口与所有写入入口共用同一个实例，核心重启后从零算。
#[derive(Default)]
pub struct ProfileRevisions {
    revisions: RwLock<BTreeMap<String, u64>>,
}

impl ProfileRevisions {
    pub fn bump(&self, plugin_id: &str) -> u64 {
        let mut revisions = self.revisions.write().expect("Bot 资料版本锁中毒");
        if let Some(revision) = revisions.get_mut(plugin_id) {
            *revision += 1;
            return *revision;
        }
        revisions.insert(plugin_id.to_owned(), 1);
        1
    }

    pub fn get(&self, plugin_id: &str) -> u64 {
        self.revisions
            .read()
            .expect("Bot 资料版本锁中毒")
            .get(plugin_id)
            .copied()
            .unwrap_or(0)
    }
}

#[derive(Default)]
pub(in crate::plugin) struct EditWindow {
    successes: VecDeque<Instant>,
}

impl EditWindow {
    pub(super) fn check(&mut self, now: Instant) -> Result<(), ProfileError> {
        while self
            .successes
            .front()
            .is_some_and(|at| now.duration_since(*at) >= WINDOW)
        {
            self.successes.pop_front();
        }
        if self.successes.len() < MAX_SUCCESSES {
            return Ok(());
        }
        let remaining = WINDOW - now.duration_since(*self.successes.front().expect("已到次数上限"));
        Err(ProfileError::RateLimited {
            retry_after_secs: remaining.as_secs() + u64::from(remaining.subsec_nanos() != 0),
        })
    }

    pub(super) fn succeeded(&mut self) {
        self.successes.push_back(Instant::now());
    }
}

#[derive(Default)]
pub(in crate::plugin) struct ProfileState {
    pub(super) revisions: Arc<ProfileRevisions>,
    edits: Mutex<BTreeMap<PluginId, Arc<AsyncMutex<EditWindow>>>>,
}

impl ProfileState {
    pub(super) fn lock(&self, id: &PluginId) -> Arc<AsyncMutex<EditWindow>> {
        let mut edits = self.edits.lock().expect("Bot 资料编辑锁中毒");
        if let Some(lock) = edits.get(id) {
            return lock.clone();
        }
        let lock = Arc::new(AsyncMutex::default());
        edits.insert(id.clone(), lock.clone());
        lock
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn successful_edits_expire_individually_at_the_sliding_window_boundary() {
        let now = Instant::now();
        let mut window = EditWindow::default();
        for second in 0..10 {
            window
                .successes
                .push_back(now + Duration::from_secs(second));
        }
        assert!(matches!(
            window.check(now + WINDOW - Duration::from_millis(1)),
            Err(ProfileError::RateLimited {
                retry_after_secs: 1
            })
        ));
        window.check(now + WINDOW).unwrap();
        assert_eq!(window.successes.len(), 9);
        window.successes.push_back(now + WINDOW);
        assert!(matches!(
            window.check(now + WINDOW),
            Err(ProfileError::RateLimited {
                retry_after_secs: 1
            })
        ));
        window.check(now + WINDOW + Duration::from_secs(1)).unwrap();
    }
}
