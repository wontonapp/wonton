//! Bot 存文件（ADR-0027）在阻塞线程上的那部分：列出共享文件夹、暂存、放文件。判定和卡片在总线。

use std::path::PathBuf;
use std::sync::Arc;

use wonton_protocol::FileFailure;

use super::share_fs::saving::{Outgoing, Placed, SaveTarget, Staged};
use super::uploads::has_room;
use super::{Failure, Files};

impl Files {
    pub async fn share_names(self: &Arc<Self>) -> Result<Vec<String>, Failure> {
        self.blocking(|files| Ok(files.shares.names()?)).await
    }

    /// 查好共享文件夹和空间，把 outbox 里的文件复制进暂存。
    pub async fn stage_save(
        self: &Arc<Self>,
        app_data: PathBuf,
        paths: Vec<String>,
        target: &SaveTarget,
    ) -> Result<Staged, Failure> {
        let share = target.share.clone();
        self.blocking(move |files| {
            files.shares.check_share(&share)?;
            let outgoing = Outgoing::open(&app_data, &paths)?;
            let total = outgoing.total();
            let share_dir = files.shares.root().join(&share);
            for dir in [&files.saving_dir, &share_dir] {
                if !has_room(dir, total)? {
                    return Err(Failure::new(
                        FileFailure::StorageFull,
                        format!("放不下这 {total} 字节，磁盘要留 5% 空闲"),
                    ));
                }
            }
            Ok(outgoing.stage(&files.saving_dir)?)
        })
        .await
    }

    /// 放进共享文件夹，收拾暂存。
    pub async fn place_saved(
        self: &Arc<Self>,
        staged: Staged,
        target: SaveTarget,
    ) -> Result<Placed, Failure> {
        self.blocking(move |files| Ok(files.shares.place_staged(&staged, &target, files.owner)))
            .await
    }
}
