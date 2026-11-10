//! 设备上传（ADR-0020 的“上传”）：字节归 tusd，这里管 tusd 问的两件事（创建时发编号、传完后放进 `transfers`）和清理。
//!
//! 没传完的上传只有一条待完成记录（`core/files/pending/<编号>.json`），不算文件：`/_wonton/files/{id}` 查不到它。
//! 不读 tusd 的 `.info`，那是它的内部格式。
//!
//! tusd 的 pre-finish 钩子有 15 秒的硬超时，放不下跨卷复制，所以放文件走 post-finish；它不等钩子就回客户端，
//! 网关要在最后一个 PATCH 的响应之前等 [`Files::finish_upload`]，204 才等于文件可用（实测见 ADR-0020 的“验证结果”）。

mod room;
mod sweep;

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub(super) use room::{has_room, out_of_inodes, quota};
use serde::{Deserialize, Serialize};
use wonton_protocol::{FileFailure, FileId, FileSource};

use super::record::{Accessor, FileRecord, Lookup, Origin};
use super::share_fs::from_io;
use super::share_fs::{Abandoned, Names, ShareError, ShareRef, Snapshot, TRANSFERS, sanitize_name};
use super::{DELETED_UNSYNCED, Failure, FileEvent, Files, OpenError};
use crate::l10n;
use crate::plugin::hostdir::{self, Owner};
use crate::plugin::languages::Languages;
use crate::plugin::manager::now_secs;
use crate::plugin::store::{
    StoreError, create_private_dir, read_json, remove_file, write_json, write_json_synced,
};

/// 这么久没有进度的上传被清掉。tusd 没有实现 tus 的 expiration，只能由核心来清。
pub const STALE_SECS: u64 = 24 * 60 * 60;
/// 传完放好、链接还没发出去的上传留这么久。授权给房间或者发到设备时不再过期（ADR-0020 的“生命周期”）。
pub const UPLOAD_TTL_SECS: u64 = 24 * 60 * 60;
/// 放下上传后磁盘上至少还要空着这个比例。
const KEEP_FREE_PERCENT: u64 = 5;
/// 客户端走了的编号记这么久：tusd 的 pre-create 钩子晚到时要认得出来，钩子不会晚这么多。
const WITHDRAWN_KEEP_SECS: u64 = 60 * 60;

/// 还没传完的上传。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Pending {
    id: FileId,
    name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    mime_type: Option<String>,
    size: u64,
    device: String,
    created_at: u64,
    /// 要放到哪个名字上、文件这时的样子，链接之前写好。作废时靠它分清名字上的文件是放下时的样子，还是主人后来改过的。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    placing: Option<Placing>,
    /// 作废了，清理到一半（崩溃了，或者有东西删不掉）。数据可能还是完整的，也不能再放：客户端已经被告知要重传，放了就多一份。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    abandoned: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Placing {
    name: String,
    snapshot: Snapshot,
}

impl Pending {
    /// 放在 `name` 上的文件的记录。`name` 是目录里现在的名字，主人和插件都能改，显示给人看的要和别的不可信的名字一样清洗；引用保留原样，
    /// 否则找不到文件。链接没发出去之前一直带过期时间，授权给房间或者发到设备时清掉。
    fn record(&self, name: String, placed: Snapshot) -> FileRecord {
        FileRecord {
            id: self.id.clone(),
            name: sanitize_name(&name),
            mime_type: self.mime_type.clone(),
            declared_size: Some(self.size),
            source: FileSource::Device,
            origin: Origin::Upload {
                reference: ShareRef {
                    share: TRANSFERS.to_owned(),
                    path: vec![name],
                },
                placed,
            },
            access: [Accessor::Owner].into(),
            owner_granted: Default::default(),
            registered: None,
            created_at: now_secs(),
            expires_at: Some(now_secs().saturating_add(UPLOAD_TTL_SECS)),
        }
    }
}

/// 创建上传时 tusd 转来的信息，都是客户端声明的，编号除外（网关定的）。
#[derive(Debug, Clone, Copy)]
pub struct NewUpload<'a> {
    pub id: &'a FileId,
    pub name: Option<&'a str>,
    pub mime_type: Option<&'a str>,
    /// 声明的长度；`Upload-Defer-Length` 时为空。
    pub size: Option<u64>,
    pub device: &'a str,
}

/// 一个编号对上传接口来说是什么。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UploadState {
    /// `device` 是创建它的设备，只有它能接着传。
    Pending {
        size: u64,
        device: String,
    },
    Finished {
        size: u64,
    },
    Unknown,
}

/// 轮到自己操作这个上传了：放文件、取消、删除都排同一个队，不能交错。离开时（包括请求断了）才从表里摘掉。
pub struct UploadTurn<'a> {
    uploads: &'a Uploads,
    id: FileId,
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

/// tusd 的目录和待完成记录。
pub(super) struct Uploads {
    /// tusd 存字节的目录，属主 `PUID:PGID`。
    dir: PathBuf,
    pending: PathBuf,
    owner: Owner,
    /// 正在放的上传：钩子和网关都会要放同一个上传，只放一次。
    finishing: Mutex<HashMap<FileId, Arc<tokio::sync::Mutex<()>>>>,
    /// “客户端来过”和“判定很久没有进度、标为作废”不能交错：HEAD、PATCH 不排上传的队（问进度不用等放文件），暂停很久的上传续传时正好碰上
    /// 清理，客户端就白传了。里面是正在作废的上传。短锁，只包这两步，不包作废本身。
    abandoning: Mutex<HashSet<FileId>>,
    /// 客户端拿到编号之前就走了的上传和当时的时间。tusd 的 pre-create 钩子可能还在路上，它写下待完成记录的话要白占 24 小时。
    withdrawn: Mutex<HashMap<FileId, u64>>,
    /// 测试用：拿着写锁时，清理在标为作废之后卡住，就像存储不回答。
    #[cfg(test)]
    abandon_stalled: tokio::sync::RwLock<()>,
}

impl Uploads {
    pub(super) fn open(files_dir: &Path, owner: Owner) -> Result<Self, OpenError> {
        let dir = files_dir.join("uploads");
        hostdir::ensure_owned_dir(&dir, owner)?;
        let pending = files_dir.join("pending");
        create_private_dir(&pending).map_err(super::RegistryError::from)?;
        Ok(Self {
            dir,
            pending,
            owner,
            finishing: Mutex::default(),
            abandoning: Mutex::default(),
            withdrawn: Mutex::default(),
            #[cfg(test)]
            abandon_stalled: tokio::sync::RwLock::default(),
        })
    }

    fn pending_path(&self, id: &FileId) -> PathBuf {
        self.pending.join(format!("{id}.json"))
    }

    fn load_pending(&self, id: &FileId) -> Result<Option<Pending>, Failure> {
        read_json(&self.pending_path(id)).map_err(|e| Failure::internal(e.to_string()))
    }

    /// 删掉 tusd 留下的文件，都删掉了才为真。`.lock` 只有清理时才删：放文件时 tusd 可能还拿着它。
    fn remove_data(&self, id: &str, with_lock: bool) -> bool {
        let suffixes: &[&str] = if with_lock {
            &["", ".info", ".lock"]
        } else {
            &["", ".info"]
        };
        let mut removed = true;
        for suffix in suffixes {
            let path = self.dir.join(format!("{id}{suffix}"));
            if let Err(e) = fs::remove_file(&path)
                && e.kind() != ErrorKind::NotFound
            {
                tracing::warn!(path = %path.display(), error = %e, "删除上传的数据失败");
                removed = false;
            }
        }
        removed
    }

    pub(super) async fn turn(&self, id: &FileId) -> UploadTurn<'_> {
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
    fn seen(&self, id: &FileId) -> bool {
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

    fn withdraw(&self, id: &FileId) {
        let now = now_secs();
        let mut withdrawn = self.withdrawn.lock().expect("上传编号表锁中毒");
        withdrawn.retain(|_, at| now.saturating_sub(*at) < WITHDRAWN_KEEP_SECS);
        withdrawn.insert(id.clone(), now);
    }

    fn take_withdrawn(&self, id: &FileId) -> bool {
        let mut withdrawn = self.withdrawn.lock().expect("上传编号表锁中毒");
        withdrawn.remove(id).is_some()
    }

    /// 传到一半 tusd 出错时，磁盘是不是真的放不下剩下的字节。
    fn out_of_space(&self, id: &FileId, langs: &Languages) -> Option<Failure> {
        let pending = self.load_pending(id).ok()??;
        let written = fs::symlink_metadata(self.dir.join(id.as_str())).map_or(0, |meta| meta.len());
        self.cannot_hold(pending.size.saturating_sub(written), langs)
    }

    fn cannot_hold(&self, left: u64, langs: &Languages) -> Option<Failure> {
        let storage_full = |key: &str| {
            Some(Failure::new(
                FileFailure::StorageFull,
                l10n::text(langs, key, &[]),
            ))
        };
        if let Some(quota) = quota(&self.dir, self.owner.uid) {
            if quota.space_left.is_some_and(|room| room < left) {
                return storage_full("files-upload-quota-full");
            }
            if quota.inodes_left == Some(0) {
                return storage_full("files-upload-inodes-full");
            }
        }
        let stats = rustix::fs::statvfs(&self.dir).ok()?;
        if out_of_inodes(stats.f_files, stats.f_favail) {
            return storage_full("files-upload-inodes-full");
        }
        let free = stats.f_bavail.saturating_mul(stats.f_frsize.max(1));
        (free < left).then(|| {
            Some(Failure::new(
                FileFailure::StorageFull,
                l10n::text(
                    langs,
                    "files-upload-leftover",
                    &[
                        ("free", wonton_l10n::Arg::from(free)),
                        ("left", wonton_l10n::Arg::from(left)),
                    ],
                ),
            ))
        })?
    }
}

impl Files {
    /// 创建上传（tusd 的 pre-create）：查磁盘、发编号。没传完的编号查不到，所以这时不写编号记录。
    pub async fn create_upload(
        self: &Arc<Self>,
        upload: NewUpload<'_>,
        langs: &Languages,
    ) -> Result<FileId, Failure> {
        let invalid = |why: &str| Failure::new(FileFailure::InvalidInput, why);
        // 排队：查“客户端走了没有”和写记录不能夹在 [`Self::upload_unclaimed`] 中间。
        let _turn = self.uploads.turn(upload.id).await;
        if self.uploads.take_withdrawn(upload.id) {
            return Err(invalid(&l10n::text(
                langs,
                "files-upload-disconnected",
                &[],
            )));
        }
        let size = upload
            .size
            .ok_or_else(|| invalid(&l10n::text(langs, "files-upload-length-required", &[])))?;
        let name = upload
            .name
            .ok_or_else(|| invalid(&l10n::text(langs, "files-upload-filename-required", &[])))?;
        let pending = Pending {
            id: upload.id.clone(),
            name: sanitize_name(name),
            mime_type: upload.mime_type.map(str::to_owned),
            size,
            device: upload.device.to_owned(),
            created_at: now_secs(),
            placing: None,
            abandoned: false,
        };
        let langs = langs.clone();
        self.blocking(move |files| {
            for dir in [
                files.uploads.dir.as_path(),
                files.shares.root().join(TRANSFERS).as_path(),
            ] {
                if !has_room(dir, size, files.uploads.owner.uid)? {
                    return Err(Failure::new(
                        FileFailure::StorageFull,
                        l10n::text(
                            &langs,
                            "files-upload-disk-full",
                            &[
                                ("size", wonton_l10n::Arg::from(size)),
                                ("percent", wonton_l10n::Arg::from(KEEP_FREE_PERCENT as i64)),
                            ],
                        ),
                    ));
                }
            }
            write_json(&files.uploads.pending_path(&pending.id), &pending)
                .map_err(|e| Failure::internal(e.to_string()))?;
            Ok(pending.id)
        })
        .await
    }

    pub async fn upload_state(self: &Arc<Self>, id: &FileId) -> Result<UploadState, Failure> {
        let id = id.clone();
        self.blocking(move |files| files.state_now(&id, || ()))
            .await
    }

    /// 不排队：问进度不用等放文件。放文件先写记录、后删待完成记录，所以待完成记录没了就再看一次记录，否则正好放完时会说没有这个
    /// 上传，客户端就重传了。先看记录：放好之后、删掉待完成记录之前核心可能停了。
    fn state_now(&self, id: &FileId, between: impl FnOnce()) -> Result<UploadState, Failure> {
        let finished = || match self.registry.get(id, now_secs()) {
            Lookup::Found(FileRecord {
                origin: Origin::Upload { placed, .. },
                declared_size,
                ..
            }) => Some(UploadState::Finished {
                // tus 规定长度创建后不变。主人后来改过（作废时补的记录）也回声明的长度。
                size: declared_size.unwrap_or(placed.size),
            }),
            _ => None,
        };
        if let Some(state) = finished() {
            return Ok(state);
        }
        between();
        if let Some(pending) = self.uploads.load_pending(id)?
            && !pending.abandoned
        {
            return Ok(UploadState::Pending {
                size: pending.size,
                device: pending.device,
            });
        }
        Ok(finished().unwrap_or(UploadState::Unknown))
    }

    /// 客户端来问进度或者接着传了。为假时这个上传正在被清掉，接不上了。
    pub async fn upload_seen(self: &Arc<Self>, id: &FileId) -> bool {
        let files = self.clone();
        let id = id.clone();
        tokio::task::spawn_blocking(move || files.uploads.seen(&id))
            .await
            .unwrap_or_else(|e| {
                tracing::warn!(error = %e, "记上传进度的任务中止");
                true
            })
    }

    /// tusd 在传的途中回了 500：真是磁盘满了就说清楚，别的原因不猜。
    pub async fn upload_out_of_space(
        self: &Arc<Self>,
        id: &FileId,
        langs: &Languages,
    ) -> Option<Failure> {
        let (id, langs) = (id.clone(), langs.clone());
        let files = self.clone();
        tokio::task::spawn_blocking(move || files.uploads.out_of_space(&id, &langs))
            .await
            .ok()?
    }

    /// 排上这个上传的队。
    pub async fn upload_turn(&self, id: &FileId) -> UploadTurn<'_> {
        self.uploads.turn(id).await
    }

    /// 传完后放进 `transfers`，这时文件才算可用。可以重复调。跑在单独的任务里：调用它的请求断了也要做完。
    pub async fn finish_upload(
        self: &Arc<Self>,
        id: &FileId,
        langs: &Languages,
    ) -> Result<FileRecord, Failure> {
        let files = self.clone();
        let (task, langs) = (id.clone(), langs.clone());
        tokio::spawn(async move {
            let _turn = files.uploads.turn(&task).await;
            files.place(&task, &langs).await
        })
        .await
        .map_err(|e| Failure::internal(format!("放上传文件的任务中止：{e}")))?
    }

    /// tusd 说它没有这个上传了。可能是刚放好（放好之后 tusd 的数据就删了），这时要照实说放好了，否则客户端重传，多出一份文件；
    /// 还没放好就是数据真的没了，这次上传接不上了。回 [`UploadState::Unknown`] 只表示客户端再也看不到这个上传了：作废时删不掉的残留
    /// 留给下次清理，取消也照样算成功。作废时可能发现放下的文件主人改过、留下了（[`Self::settle_placed`]），所以再查一次，如实回答。
    pub async fn settle_lost_upload(
        self: &Arc<Self>,
        turn: &UploadTurn<'_>,
    ) -> Result<UploadState, Failure> {
        match self.upload_state(&turn.id).await? {
            UploadState::Pending { .. } => {
                self.abandon_upload(&turn.id).await;
                Ok(match self.upload_state(&turn.id).await? {
                    UploadState::Pending { .. } => UploadState::Unknown,
                    state => state,
                })
            }
            state => Ok(state),
        }
    }

    /// 这个上传不要了：客户端取消了，或者客户端拿不到编号、再也接不上了。
    pub async fn upload_cancelled(self: &Arc<Self>, turn: &UploadTurn<'_>) {
        self.abandon_upload(&turn.id).await;
    }

    /// 客户端拿到编号之前就走了：清掉，tusd 的钩子晚到的话也不再记下它。
    pub async fn upload_unclaimed(self: &Arc<Self>, turn: &UploadTurn<'_>) {
        self.abandon_upload(&turn.id).await;
        self.uploads.withdraw(&turn.id);
    }

    async fn place(
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
    fn place_pending(&self, pending: Pending, langs: &Languages) -> Result<FileRecord, Failure> {
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
    fn place_marked(
        &self,
        pending: &Pending,
        langs: &Languages,
    ) -> Result<(String, Snapshot), ShareError> {
        let mut marked = pending.clone();
        let placed = self.shares.place_upload(
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
        let temps = match self.shares.upload_temps() {
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

    /// 上传作废了：客户端取消了，tusd 没有它了，或者很久没有进度。调用方排着这个上传的队。
    async fn abandon_upload(self: &Arc<Self>, id: &FileId) {
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
    fn abandon_now(&self, id: &FileId) -> bool {
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

    fn mark_abandoned(&self, id: &FileId) -> Result<(), Failure> {
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
        let Some(pending) = self.uploads.load_pending(id)? else {
            return Ok(());
        };
        let placing = pending.placing.as_ref();
        let placing = placing.map(|p| (p.name.as_str(), &p.snapshot));
        // 作废是后台清理，没有请求头：里面的错误按英文说。
        let langs = Languages::from_header(None);
        match self.shares.settle_abandoned(id.as_str(), placing, &langs)? {
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
    pub(super) fn forget_now(&self, id: &FileId, with_lock: bool) -> bool {
        let temp = self.shares.remove_upload_temp(id.as_str());
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

#[cfg(test)]
impl Files {
    /// 钩子问过核心之后的样子：只有待完成记录。磁盘放不放得下不查。
    pub(crate) fn pending_for_test(&self, id: &FileId, size: u64, device: &str) {
        let pending = Pending {
            id: id.clone(),
            name: "a.txt".to_owned(),
            mime_type: None,
            size,
            device: device.to_owned(),
            created_at: now_secs(),
            placing: None,
            abandoned: false,
        };
        write_json(&self.uploads.pending_path(id), &pending).unwrap();
    }

    /// 不排队地放：测试拿着队，模拟放到一半。
    pub(crate) async fn place_for_test(
        self: &Arc<Self>,
        id: &FileId,
        langs: &Languages,
    ) -> Result<FileRecord, Failure> {
        self.place(id, langs).await
    }
}

#[cfg(test)]
mod abandon_tests;
#[cfg(test)]
mod expiry_tests;
#[cfg(test)]
mod quota_tests;
#[cfg(test)]
mod tests;
