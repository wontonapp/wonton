//! 上传的队：放文件、取消、删除排同一个队，不能交错；客户端来过的进度和拿编号之前就走了的上传也记在这里。

use std::fs;
use std::io::ErrorKind;
use std::sync::Arc;

use wonton_protocol::FileId;

use super::{Uploads, WITHDRAWN_KEEP_SECS};
use crate::plugin::manager::now_secs;

/// 轮到自己操作这个上传了：放文件、取消、删除都排同一个队，不能交错。离开时（包括请求断了）才从表里摘掉。
pub struct UploadTurn<'a> {
    uploads: &'a Uploads,
    pub(super) id: FileId,
    lock: Arc<tokio::sync::Mutex<()>>,
    guard: Option<tokio::sync::OwnedMutexGuard<()>>,
}

impl Drop for UploadTurn<'_> {
    /// 还有人在排队就留在表里，后来的调用才会和他们排同一个队。引用都在表锁里拿，所以计数是准的（表里一份，自己一份）。
    fn drop(&mut self) {
        self.guard = None;
        let mut finishing = self.uploads.finishing.lock().expect("上传锁表中毒");
        if Arc::strong_count(&self.lock) == 2 {
            finishing.remove(&self.id);
        }
    }
}

impl Uploads {
    pub(in crate::files) async fn turn(&self, id: &FileId) -> UploadTurn<'_> {
        let lock = {
            let mut finishing = self.finishing.lock().expect("上传锁表中毒");
            finishing.entry(id.clone()).or_default().clone()
        };
        let mut turn = UploadTurn {
            uploads: self,
            id: id.clone(),
            lock: lock.clone(),
            guard: None,
        };
        turn.guard = Some(lock.lock_owned().await);
        turn
    }

    /// 客户端来过：待完成记录的修改时间算最后一次见到它。tusd 写第一块之前数据的修改时间不变，只看数据的话，暂停很久以后接着传时会被清掉。
    /// 不用 tusd 的 `<编号>.lock`：tusd 崩溃后它会留下来，这个上传就永远不清了。没有记录就不算数，不新建。已经在作废了就晚了，为假。
    pub(super) fn seen(&self, id: &FileId) -> bool {
        let abandoning = self.abandoning.lock().expect("上传进度锁中毒");
        if abandoning.contains(id) {
            return false;
        }
        let touched = fs::File::options()
            .write(true)
            .open(self.pending_path(id))
            .and_then(|record| record.set_modified(std::time::SystemTime::now()));
        match touched {
            Err(e) if e.kind() != ErrorKind::NotFound => {
                tracing::warn!(file = %id, error = %e, "记不下上传的进度");
            }
            _ => {}
        }
        true
    }

    pub(super) fn withdraw(&self, id: &FileId) {
        let now = now_secs();
        let mut withdrawn = self.withdrawn.lock().expect("上传编号表锁中毒");
        withdrawn.retain(|_, at| now.saturating_sub(*at) < WITHDRAWN_KEEP_SECS);
        withdrawn.insert(id.clone(), now);
    }

    pub(super) fn take_withdrawn(&self, id: &FileId) -> bool {
        let mut withdrawn = self.withdrawn.lock().expect("上传编号表锁中毒");
        withdrawn.remove(id).is_some()
    }
}
