//! 队列里别的项的流地址（无缝播放、预取）：一次起播计划给一项用到过期。播放器按 `Range` 一段段取，
//! 每段都重要计划会让提供方那边开一堆上游会话（Jellyfin 的 `PlaybackInfo`），所以记在核心内存里。
//!
//! 只记提供方的路径，重启就丢：丢了无非再要一次计划。

use std::collections::HashMap;
use std::sync::Mutex;

use wonton_protocol::media::{PlaybackSessionId, Timestamp};

/// 一条会话最多记几项，超过就整条清掉重来；正常的预取只有下一两首。
const PER_SESSION: usize = 8;

#[derive(Default)]
pub(crate) struct QueuedStreams {
    plans: Mutex<HashMap<PlaybackSessionId, HashMap<String, Remembered>>>,
}

#[derive(Clone)]
struct Remembered {
    path: String,
    /// Unix 毫秒；0 是提供方没说过期
    expires_at: Timestamp,
}

impl QueuedStreams {
    /// 还没过期的那一项的路径。
    pub(crate) fn get(
        &self,
        session: PlaybackSessionId,
        item: &str,
        now: Timestamp,
    ) -> Option<String> {
        let plans = self.plans.lock().unwrap_or_else(|p| p.into_inner());
        plans
            .get(&session)?
            .get(item)
            .filter(|r| r.expires_at == 0 || r.expires_at > now)
            .map(|r| r.path.clone())
    }

    pub(crate) fn put(
        &self,
        session: PlaybackSessionId,
        item: &str,
        path: String,
        expires_at: Timestamp,
    ) {
        let mut plans = self.plans.lock().unwrap_or_else(|p| p.into_inner());
        let items = plans.entry(session).or_default();
        if items.len() >= PER_SESSION {
            items.clear();
        }
        items.insert(item.to_owned(), Remembered { path, expires_at });
    }

    /// 会话停了，它记的都不要了。
    pub(crate) fn forget(&self, session: PlaybackSessionId) {
        self.plans
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&session);
    }
}
