//! 放上传：放进 `transfers`、写记录，然后清理 tusd 留下的东西。

use std::sync::Arc;

use wonton_protocol::FileId;

use super::super::record::{FileRecord, Lookup, Origin};
use super::super::share_fs::from_io;
use super::super::share_fs::{Names, ShareError, Snapshot};
use super::{Failure, FileEvent, Files, Pending, Placing};
use crate::l10n;
use crate::plugin::languages::Languages;
use crate::plugin::manager::now_secs;
use crate::plugin::store::{StoreError, write_json_synced};

impl Files {
    pub(super) async fn place(
        self: &Arc<Self>,
        id: &FileId,
        langs: &Languages,
    ) -> Result<FileRecord, Failure> {
        if let Lookup::Found(record) = self.registry.get(id, now_secs())
            && matches!(record.origin, Origin::Upload { .. })
        {
            // 写好记录之后没来得及清理。
            self.clean_up_placed(id).await;
            return Ok(record);
        }
        let (task, langs) = (id.clone(), langs.clone());
        let pending = self
            .blocking(move |files| files.uploads.load_pending(&task))
            .await?
            .ok_or_else(|| Failure::unknown(&langs))?;
        if pending.abandoned {
            self.abandon_upload(id).await;
            return Err(Failure::unknown(&langs));
        }
        let device = pending.device.clone();
        let langs = langs.clone();
        let placed = self
            .blocking(move |files| files.place_pending(pending, &langs))
            .await;
        self.audit(&FileEvent {
            event: "file_upload",
            device,
            file: id.as_str(),
            range: None,
            bots: None,
            outcome: placed.as_ref().map_or_else(|e| e.kind.as_str(), |_| "ok"),
        });
        placed
    }

    /// 放文件、写记录、清理，这个顺序：数据一直留到记录写好。任何一步失败或者崩溃，文件都还认得出来（`place_upload` 按 inode 认），
    /// 再来一次会收敛到同一个文件和同一条记录。
    pub(super) fn place_pending(
        &self,
        pending: Pending,
        langs: &Languages,
    ) -> Result<FileRecord, Failure> {
        let _home = self
            .home_changes
            .read()
            .expect("home changes lock poisoned");
        self.host.check_person(&pending.accessor)?;
        let (name, placed) = self.place_marked(&pending, langs)?;
        let record = pending.record(name, placed);
        // 记录的目录项没落盘时不清理：断电后记录没了，数据还在才放得回来。下次清理再来。
        if self.registry.save_synced(record.clone())? {
            self.forget_now(&pending.id, false);
        }
        Ok(record)
    }

    /// 放文件，链接之前把名字和样子写进待完成记录。写不了或者没落盘就不放：之后分不清名字上的文件是谁的。`fsync` 的错误只报告一次，所以重试
    /// 时这里会重新写一次。
    pub(super) fn place_marked(
        &self,
        pending: &Pending,
        langs: &Languages,
    ) -> Result<(String, Snapshot), ShareError> {
        let mut marked = pending.clone();
        let shares = self
            .home_shares(&pending.username, false)
            .map_err(|e| ShareError::Io(e.to_string()))?;
        let placed = shares.place_upload(
            &self.uploads.dir,
            pending.id.as_str(),
            pending.size,
            Names {
                preferred: &pending.name,
                owner: self.uploads.owner,
                placing_inode: pending.placing.as_ref().map(|p| p.snapshot.inode),
            },
            langs,
            &mut |name, snapshot| {
                marked.placing = Some(Placing {
                    name: name.to_owned(),
                    snapshot: *snapshot,
                });
                let path = self.uploads.pending_path(&pending.id);
                let label = l10n::text(langs, "files-upload-record-label", &[]);
                let synced = write_json_synced(&path, &marked).map_err(|e| match e {
                    StoreError::Io { source, .. } => from_io(&source, &label),
                    other => ShareError::Io(other.to_string()),
                })?;
                if !synced {
                    return Err(ShareError::Io(l10n::text(
                        langs,
                        "files-upload-record-unwritable",
                        &[],
                    )));
                }
                Ok(())
            },
        );
        if placed.is_err() {
            self.forget_failed_placement(&pending.id);
        }
        placed
    }

    /// 链接、复制失败后临时文件已经删掉了：记下的 inode 会被别的文件复用（跨卷的临时文件是复制出来的，inode 和下一条记录无关），
    /// 不清掉的话下次会把它当成放好的那个。公开名字上不是这个文件才会走到这里（[`Shares::place_upload`] 只在失败后删临时文件）。
    fn forget_failed_placement(&self, id: &FileId) {
        // 调用前的克隆里没有这一趟写下的放置信息，从盘上重新读。
        let Some(mut current) = self.uploads.load_pending(id).ok().flatten() else {
            return;
        };
        if current.placing.is_none() {
            return;
        }
        let temps = match self
            .home_shares(&current.username, false)
            .and_then(|shares| shares.upload_temps().map_err(Failure::from))
        {
            Ok(temps) => temps,
            Err(e) => {
                tracing::warn!(file = %id, error = %e, "读不了临时文件，放置记录留着");
                return;
            }
        };
        if temps.iter().any(|temp| temp == id.as_str()) {
            return;
        }
        current.placing = None;
        match write_json_synced(&self.uploads.pending_path(id), &current) {
            Ok(true) => {}
            Ok(false) => tracing::warn!(file = %id, "清掉的放置记录没能落盘"),
            Err(e) => tracing::warn!(file = %id, error = %e, "清掉放置记录失败"),
        }
    }

    /// 传完放好了，不再管它。调用方排着这个上传的队。
    async fn clean_up_placed(self: &Arc<Self>, id: &FileId) {
        let files = self.clone();
        let id = id.clone();
        let cleared = tokio::task::spawn_blocking(move || match files.registry.resave(&id) {
            Ok(true) => {
                files.forget_now(&id, false);
            }
            Ok(false) => {}
            Err(e) => tracing::warn!(file = %id, error = %e, "重写上传的记录失败，下次清理再试"),
        })
        .await;
        if let Err(e) = cleared {
            tracing::warn!(error = %e, "清理上传的任务中止");
        }
    }
}
