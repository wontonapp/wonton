//! 定期清理：放好传完但没放的上传，清掉没有进度的上传和过期够久的编号。

use std::collections::HashSet;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use wonton_protocol::FileId;

use super::{Pending, STALE_SECS, Uploads};
use crate::files::record::Lookup;
use crate::files::{Failure, FileEvent, Files};
use crate::plugin::languages::Languages;
use crate::plugin::manager::now_secs;
use crate::plugin::store::{StoreError, read_json};

const SWEEP_EVERY: Duration = Duration::from_secs(60 * 60);
/// 读不懂的待完成记录改成这个后缀。
const CORRUPT_SUFFIX: &str = ".json.corrupt";

/// 一次清理查到的。
#[derive(Default)]
pub(super) struct Scan {
    /// 传完了还没放（钩子没到核心，比如核心重启了）。
    pub(super) complete: Vec<FileId>,
    /// 数据没传完的。
    incomplete: Vec<FileId>,
    /// 数据没传完，也很久没有进度了。
    stale: Vec<FileId>,
    /// 作废了，上次没清完。
    abandoned: Vec<FileId>,
}

impl Uploads {
    /// 查待完成记录和 tusd 的目录，清掉没有进度的。一项出错只记日志，别的照常：一个坏文件不能让传完的上传一直放不进去。
    pub(super) fn scan(&self, now: u64) -> Scan {
        let mut scan = Scan::default();
        let entries = match fs::read_dir(&self.pending) {
            Ok(entries) => entries,
            Err(e) => {
                tracing::error!(path = %self.pending.display(), error = %e, "读不了待完成的上传记录");
                return scan;
            }
        };
        let mut alive = HashSet::new();
        let mut all_seen = true;
        for entry in entries {
            match entry {
                Ok(entry) => self.scan_pending(&entry.path(), now, &mut alive, &mut scan),
                Err(e) => {
                    tracing::error!(error = %e, "读待完成的上传记录失败");
                    all_seen = false;
                }
            }
        }
        // 没看全就不知道哪些数据还有人要，不删。
        if all_seen {
            self.remove_orphans(now, &alive);
        }
        scan
    }

    fn scan_pending(&self, path: &Path, now: u64, alive: &mut HashSet<String>, scan: &mut Scan) {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
        let name = name.unwrap_or_default();
        let stem = name.split('.').next().unwrap_or_default().to_owned();
        if name.ends_with(CORRUPT_SUFFIX) {
            alive.insert(stem);
            return;
        }
        if !name.ends_with(".json") {
            // 写到一半崩溃留下的临时文件：tusd 还没拿到这个编号，也就没有数据。
            let stale = fs::symlink_metadata(path)
                .is_ok_and(|meta| now.saturating_sub(mtime_secs(&meta)) >= STALE_SECS);
            if stale && let Err(e) = fs::remove_file(path) {
                tracing::warn!(path = %path.display(), error = %e, "删除临时文件失败");
            }
            return;
        }
        let pending: Pending = match read_json(path) {
            Ok(Some(pending)) => pending,
            Ok(None) => return,
            Err(e @ StoreError::Corrupt { .. }) => {
                alive.insert(stem);
                quarantine(path, &e);
                return;
            }
            Err(e) => {
                alive.insert(stem);
                tracing::error!(error = %e, "读待完成的上传记录失败，下次清理时再试");
                return;
            }
        };
        alive.insert(pending.id.to_string());
        if pending.abandoned {
            scan.abandoned.push(pending.id);
            return;
        }
        let (finished, stale) = self.progress(&pending, now);
        if finished {
            scan.complete.push(pending.id);
        } else if stale {
            scan.stale.push(pending.id);
        } else {
            scan.incomplete.push(pending.id);
        }
    }

    /// 数据传完了吗；没传完的话，是不是很久没有进度了。
    pub(super) fn progress(&self, pending: &Pending, now: u64) -> (bool, bool) {
        let data = fs::symlink_metadata(self.dir.join(pending.id.as_str()));
        let seen = fs::symlink_metadata(self.pending_path(&pending.id));
        let last_seen = data
            .as_ref()
            .map_or(pending.created_at, mtime_secs)
            .max(seen.as_ref().map_or(0, mtime_secs));
        let finished = data
            .as_ref()
            .is_ok_and(|meta| meta.is_file() && meta.len() == pending.size);
        (
            finished,
            !finished && now.saturating_sub(last_seen) >= STALE_SECS,
        )
    }

    /// 删掉 tusd 目录里没有待完成记录、放了很久的文件。
    fn remove_orphans(&self, now: u64, alive: &HashSet<String>) {
        let entries = match fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            Err(e) => {
                tracing::error!(path = %self.dir.display(), error = %e, "读不了 tusd 的目录");
                return;
            }
        };
        for entry in entries {
            let removed = entry.and_then(|entry| {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if alive.contains(name.split('.').next().unwrap_or_default()) {
                    return Ok(());
                }
                let meta = entry.metadata()?;
                if meta.is_file() && now.saturating_sub(mtime_secs(&meta)) >= STALE_SECS {
                    fs::remove_file(entry.path())?;
                }
                Ok(())
            });
            if let Err(e) = removed {
                tracing::warn!(error = %e, "清理 tusd 目录里的一项失败，下次再试");
            }
        }
    }
}

/// 记录是原子写的，读不懂说明出了别的问题。不猜：改名留给主人看（只报一次错），它的数据不再自动清理。这次上传查不到了，客户端
/// 收到 `file_unknown` 后重传。
fn quarantine(path: &Path, why: &StoreError) {
    let mut target = path.as_os_str().to_owned();
    target.push(CORRUPT_SUFFIX.trim_start_matches(".json"));
    let target = PathBuf::from(target);
    match fs::rename(path, &target) {
        Ok(()) => tracing::error!(
            error = %why,
            kept = %target.display(),
            "待完成的上传记录损坏，已留作检查；这次上传的数据不再自动清理，要手动删"
        ),
        Err(e) => {
            tracing::error!(error = %why, rename_error = %e, "待完成的上传记录损坏，也改不了名")
        }
    }
}

fn mtime_secs(meta: &fs::Metadata) -> u64 {
    u64::try_from(meta.mtime()).unwrap_or(0)
}

impl Files {
    /// 清掉很久没有进度的上传。排队期间可能又有了进度，所以再查一次，查和标为作废不能和“客户端来过”交错。作废本身放在锁外面：要动
    /// `transfers`，NAS 卡住时不能让所有上传的 HEAD、PATCH 跟着等。做不完就去掉标记，待完成记录还在，下次清理再判。
    fn clear_if_stale(&self, id: &FileId, now: u64) -> Result<(), Failure> {
        {
            let mut abandoning = self.uploads.abandoning.lock().expect("上传进度锁中毒");
            let Some(pending) = self.uploads.load_pending(id)? else {
                return Ok(());
            };
            if !self.uploads.progress(&pending, now).1 {
                return Ok(());
            }
            abandoning.insert(id.clone());
        }
        #[cfg(test)]
        let _stalled = self.uploads.abandon_stalled.blocking_read();
        self.abandon_now(id);
        let mut abandoning = self.uploads.abandoning.lock().expect("上传进度锁中毒");
        abandoning.remove(id);
        Ok(())
    }

    /// 启动时和每小时：放好传完但没放的上传，清掉没有进度的上传和过期够久的编号。
    pub async fn sweep(self: &Arc<Self>, now: u64) {
        let scanned = self
            .blocking(move |files| {
                let scan = files.uploads.scan(now);
                // 先查到期：这一趟里作废旧上传时补的记录（主人改过的文件）要等下个钟头，不在同一次清理里马上到期。
                let expired = files.registry.expired_uploads(now);
                let mut retry = scan.complete;
                // 记录写好了，清理没做完：也走一遍，那边会接着清。
                retry.extend(
                    scan.incomplete
                        .into_iter()
                        .filter(|id| matches!(files.registry.get(id, now), Lookup::Found(_))),
                );
                Ok((retry, scan.stale, scan.abandoned, expired))
            })
            .await;
        let (retry, stale, abandoned, expired) = scanned.unwrap_or_else(|e| {
            tracing::error!(error = %e, "清理上传目录的任务中止");
            Default::default()
        });
        for id in abandoned {
            let _turn = self.uploads.turn(&id).await;
            self.abandon_upload(&id).await;
        }
        for id in stale {
            let _turn = self.uploads.turn(&id).await;
            let task = id.clone();
            let cleared = self
                .blocking(move |files| files.clear_if_stale(&task, now))
                .await;
            if let Err(e) = cleared {
                tracing::warn!(file = %id, error = %e, "清理没有进度的上传失败，下次再试");
            }
        }
        for id in retry {
            // 清理是后台任务，没有请求头。
            let langs = Languages::from_header(None);
            if let Err(e) = self.finish_upload(&id, &langs).await {
                tracing::error!(file = %id, error = %e, "放传完的文件失败，下次清理时再试");
            }
        }
        for id in expired {
            let _turn = self.uploads.turn(&id).await;
            let task = id.clone();
            let deleted = self
                .blocking(move |files| files.delete_expired_now(&task, now))
                .await;
            match deleted {
                Ok(outcome) if outcome != "kept" => self.audit(&FileEvent {
                    event: "file_delete",
                    device: "expiry".to_owned(),
                    file: id.as_str(),
                    range: None,
                    bots: None,
                    outcome,
                }),
                Ok(_) => {}
                Err(e) => tracing::warn!(file = %id, error = %e, "删过期的上传失败，下次再试"),
            }
        }
        self.remove_orphan_temps().await;
        let purged = self
            .blocking(move |files| Ok(files.registry.purge_expired(now)?))
            .await;
        if let Err(e) = purged {
            tracing::error!(error = %e, "清理过期的编号失败");
        }
    }

    /// 清掉没有待完成记录的临时文件。名字要是一个合法的编号才算：主人可能有名字相近的文件。
    async fn remove_orphan_temps(self: &Arc<Self>) {
        let temps = self
            .blocking(move |files| {
                let root = files.data_dir.join("core/home");
                let entries = match fs::read_dir(root) {
                    Ok(entries) => entries,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
                    Err(e) => return Err(Failure::internal(e.to_string())),
                };
                let mut temps = Vec::new();
                for entry in entries {
                    let entry = entry.map_err(|e| Failure::internal(e.to_string()))?;
                    if !entry
                        .file_type()
                        .map_err(|e| Failure::internal(e.to_string()))?
                        .is_dir()
                    {
                        continue;
                    }
                    let username = entry
                        .file_name()
                        .into_string()
                        .map_err(|_| Failure::internal("Invalid home name"))?;
                    let shares = files.home_shares(&username, false)?;
                    for id in shares.upload_temps()? {
                        temps.push((username.clone(), id));
                    }
                }
                Ok(temps)
            })
            .await;
        let temps = temps.unwrap_or_else(|e: Failure| {
            tracing::error!(error = %e, "Failed to read private upload directories for cleanup");
            Vec::new()
        });
        for (username, id) in temps
            .into_iter()
            .filter_map(|(user, id)| id.parse::<FileId>().ok().map(|id| (user, id)))
        {
            let _turn = self.uploads.turn(&id).await;
            let task = id.clone();
            let removed = self
                .blocking(move |files| {
                    if files.uploads.load_pending(&task)?.is_none() {
                        files
                            .home_shares(&username, false)?
                            .remove_upload_temp(task.as_str())?;
                    }
                    Ok(())
                })
                .await;
            if let Err(e) = removed {
                tracing::warn!(file = %id, error = %e, "清理上传的临时文件失败，下次再试");
            }
        }
    }

    /// 在后台定期 [`Self::sweep`]。
    pub fn spawn_sweeper(self: &Arc<Self>) {
        let files = self.clone();
        tokio::spawn(async move {
            let mut every = tokio::time::interval(SWEEP_EVERY);
            loop {
                every.tick().await;
                files.sweep(now_secs()).await;
            }
        });
    }
}
