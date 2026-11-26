//! pre-create 钩子还没回答的上传。tusd 等钩子有期限（`-hooks-http-timeout`，默认 15 秒），过了就回 500
//! `ERR_INTERNAL_SERVER_ERROR: context deadline exceeded`，和它自己出错时一样。网关凭这里分清：钩子还在做就是核心自己慢，
//! 不是 tusd 的错，也不是这个上传坏了。磁盘很忙时写待完成记录能卡十几秒（#647 实测时 IO 压力 full avg10 87%）。

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, PoisonError};

use wonton_protocol::FileId;

use super::super::Files;

/// 编号和正在做的钩子数：tusd 会重试钩子（`-hooks-http-retry`），同一个编号可能同时有两个。
#[derive(Default)]
pub(in crate::files) struct Creating(Mutex<HashMap<FileId, usize>>);

/// 钩子做完（不管成没成）时离开。
pub(in crate::files) struct Entered<'a> {
    creating: &'a Creating,
    id: FileId,
}

impl Creating {
    pub(in crate::files) fn enter(&self, id: &FileId) -> Entered<'_> {
        *self.lock().entry(id.clone()).or_default() += 1;
        Entered {
            creating: self,
            id: id.clone(),
        }
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<FileId, usize>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Drop for Entered<'_> {
    fn drop(&mut self) {
        let mut creating = self.creating.lock();
        if let Some(count) = creating.get_mut(&self.id) {
            *count -= 1;
            if *count == 0 {
                creating.remove(&self.id);
            }
        }
    }
}

impl Files {
    /// 这个上传的 pre-create 钩子还没回答（包括还在排队）。
    pub fn upload_being_created(&self, id: &FileId) -> bool {
        self.uploads.creating.lock().contains_key(id)
    }
}
