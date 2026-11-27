//! pre-create 钩子还没回答的上传。tusd 等钩子有期限（`-hooks-http-timeout`，默认 15 秒），过了就回 500
//! `ERR_INTERNAL_SERVER_ERROR: context deadline exceeded`，和它自己出错时一样。网关凭这里分清：钩子还在做就是核心自己慢，
//! 不是 tusd 的错，也不是这个上传坏了。磁盘很忙时写待完成记录能卡十几秒（#647 实测时 IO 压力 full avg10 87%）。
//!
//! 钩子做完以后还认 [`DONE_GRACE`]：写完记录和 tusd 的期限几乎同时的时候，网关可能在这道钩子已经做完之后才看，只认“还在做”
//! 就正好漏掉，把 tusd 的裸 500 转给客户端。做活的 handler 被丢掉时（tusd 等不及关掉钩子连接），登记由做活的任务持有
//! （见 `Files::create_upload`），也扛得过去。

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use wonton_protocol::FileId;

use super::super::Files;

/// 钩子做完以后还认多久。网关在 tusd 回 500 时看登记，离钩子做完只差调度和网络上的几毫秒；留够余量。
const DONE_GRACE: Duration = Duration::from_secs(30);

/// 一个编号的钩子：还在做的个数、最后一个做完的时刻。tusd 会重试钩子（`-hooks-http-retry`），同一个编号可能同时有两个。
#[derive(Default)]
struct Entry {
    inflight: usize,
    /// 还有在做的就是 None。
    done_at: Option<Instant>,
}

#[derive(Default)]
pub(in crate::files) struct Creating(Mutex<HashMap<FileId, Entry>>);

impl Entry {
    /// 还在做，或者刚做完（[`DONE_GRACE`] 以内）。
    fn alive(&self, now: Instant) -> bool {
        self.inflight > 0
            || self
                .done_at
                .is_some_and(|done| now.saturating_duration_since(done) < DONE_GRACE)
    }
}

/// 钩子做完（不管成没成）时离开。
pub(in crate::files) struct Entered<'a> {
    creating: &'a Creating,
    id: FileId,
}

impl Creating {
    pub(in crate::files) fn enter(&self, id: &FileId) -> Entered<'_> {
        let mut creating = self.lock();
        let now = Instant::now();
        creating.retain(|_, entry| entry.alive(now));
        let entry = creating.entry(id.clone()).or_default();
        entry.inflight += 1;
        entry.done_at = None;
        Entered {
            creating: self,
            id: id.clone(),
        }
    }

    /// 这个编号还在做，或者刚做完。
    pub(in crate::files) fn contains(&self, id: &FileId) -> bool {
        let mut creating = self.lock();
        let now = Instant::now();
        creating.retain(|_, entry| entry.alive(now));
        creating.contains_key(id)
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<FileId, Entry>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Drop for Entered<'_> {
    fn drop(&mut self) {
        let mut creating = self.creating.lock();
        if let Some(entry) = creating.get_mut(&self.id) {
            entry.inflight = entry.inflight.saturating_sub(1);
            if entry.inflight == 0 {
                entry.done_at = Some(Instant::now());
            }
        }
    }
}

impl Files {
    /// 这个上传的 pre-create 钩子还没回答（包括还在排队），或者刚回答完（[`DONE_GRACE`] 以内）。
    pub fn upload_being_created(&self, id: &FileId) -> bool {
        self.uploads.creating.contains(id)
    }
}
