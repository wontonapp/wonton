//! 上传不要了：客户端取消、tusd 没有它了、很久没有进度。先落盘标成作废，再清掉。

use std::sync::Arc;

use wonton_protocol::FileId;

use super::super::record::Lookup;
use super::super::share_fs::{Abandoned, ShareError};
use super::super::{DELETED_UNSYNCED, FileEvent};
use super::{Failure, Files};
use crate::plugin::languages::Languages;
use crate::plugin::manager::now_secs;
use crate::plugin::store::{remove_file, write_json_synced};

impl Files {
    /// 上传作废了：客户端取消了，tusd 没有它了，或者很久没有进度。调用方排着这个上传的队。
    pub(super) async fn abandon_upload(self: &Arc<Self>, id: &FileId) {
        let files = self.clone();
        let id = id.clone();
        let cleared = tokio::task::spawn_blocking(move || files.abandon_now(&id)).await;
        if let Err(e) = cleared {
            tracing::warn!(error = %e, "清理上传的任务中止");
        }
    }

    /// 有记录的不动，那是主人的文件；记录落盘了才算数，否则断电后文件没了记录，所以重写一次（同 [`Self::clean_up_placed`]）。放好了却没写成
    /// 记录的见 [`Self::settle_placed`]，先落盘标成作废：下面清到一半崩溃的话，清理看到完整的数据不会再放一次。处理不了就留着待完成记录，
    /// 下次清理再来。`fsync` 的错误只报告一次，所以每次都重写标记。
    pub(super) fn abandon_now(&self, id: &FileId) -> bool {
        let recorded = !matches!(self.registry.get(id, now_secs()), Lookup::Unknown);
        let settled = if recorded {
            self.registry
                .resave(id)
                .map_err(Failure::from)
                .and_then(|durable| {
                    durable
                        .then_some(())
                        .ok_or_else(|| Failure::internal("记录没能落盘"))
                })
        } else {
            self.mark_abandoned(id)
                .and_then(|()| self.settle_placed(id))
        };
        if let Err(e) = settled {
            tracing::warn!(file = %id, error = %e, "处理已放好的文件失败，下次清理再试");
            return false;
        }
        self.forget_now(id, true)
    }

    pub(super) fn mark_abandoned(&self, id: &FileId) -> Result<(), Failure> {
        let Some(mut pending) = self.uploads.load_pending(id)? else {
            return Ok(());
        };
        pending.abandoned = true;
        let synced = write_json_synced(&self.uploads.pending_path(id), &pending)
            .map_err(|e| Failure::internal(e.to_string()))?;
        synced
            .then_some(())
            .ok_or_else(|| Failure::internal("作废的标记没能落盘"))
    }

    /// 没有记录的文件不能留在公开的名字上：App 删不掉，清理也再找不到它。还是放下时的样子就按没放过算，删掉，不补记录：客户端取消了就是
    /// 不要了，其余两种情况客户端已经被告知要重传，留着或者补上都会多一份。主人改过的是主人的文件：补一条记录，留着。
    fn settle_placed(&self, id: &FileId) -> Result<(), Failure> {
        let _home = self
            .home_changes
            .read()
            .expect("home changes lock poisoned");
        let Some(pending) = self.uploads.load_pending(id)? else {
            return Ok(());
        };
        let placing = pending.placing.as_ref();
        let placing = placing.map(|p| (p.name.as_str(), &p.snapshot));
        // 作废是后台清理，没有请求头：里面的错误按英文说。
        let langs = Languages::from_header(None);
        let settled = self
            .home_shares(&pending.username, false)?
            .settle_abandoned(id.as_str(), placing, &langs);
        let settled = match settled {
            Err(ShareError::Gone) => Abandoned::NotPlaced,
            result => result?,
        };
        match settled {
            Abandoned::NotPlaced => {}
            Abandoned::Deleted { name, durable } => {
                tracing::warn!(file = %id, name, "上传作废了，删掉了已经放好但没有记录的文件");
                self.audit(&FileEvent {
                    event: "file_delete",
                    device: pending.device,
                    file: id.as_str(),
                    range: None,
                    bots: None,
                    outcome: if durable { "ok" } else { DELETED_UNSYNCED },
                });
                // 待完成记录删掉之后，断电回来的文件就再也没人管了。留着，下次清理再判。
                if !durable {
                    return Err(Failure::internal("删除没能确认落盘"));
                }
            }
            Abandoned::Kept { name, snapshot } => {
                tracing::warn!(file = %id, name, "上传作废了，放好的文件主人改过，留着并补上记录");
                if !self.registry.save_synced(pending.record(name, snapshot))? {
                    return Err(Failure::internal("补的记录没能落盘"));
                }
            }
        }
        Ok(())
    }

    /// 待完成记录最后删，前面都删掉了才删：它是唯一指向临时文件和数据的东西，留着下次清理还看得到这个上传。临时文件删不掉就成了
    /// 主人文件的第二个硬链接，删掉可见的文件也不释放空间。都删掉了才为真。
    pub(in crate::files) fn forget_now(&self, id: &FileId, with_lock: bool) -> bool {
        let temp = self
            .uploads
            .load_pending(id)
            .and_then(|pending| match pending {
                Some(pending) => match self
                    .home_shares(&pending.username, false)?
                    .remove_upload_temp(id.as_str())
                {
                    Ok(()) | Err(ShareError::Gone) => Ok(()),
                    Err(error) => Err(Failure::from(error)),
                },
                None => Ok(()),
            });
        if let Err(e) = &temp {
            tracing::warn!(file = %id, error = %e, "删除上传的临时文件失败");
        }
        let data = self.uploads.remove_data(id.as_str(), with_lock);
        if temp.is_err() || !data {
            return false;
        }
        match remove_file(&self.uploads.pending_path(id)) {
            Ok(()) => true,
            Err(e) => {
                tracing::warn!(error = %e, "删除上传记录失败");
                false
            }
        }
    }
}
