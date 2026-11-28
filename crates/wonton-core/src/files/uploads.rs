//! 设备上传（ADR-0020 的“上传”）：字节归 tusd，这里管 tusd 问的两件事（创建时发编号、传完后放进 `transfers`）和清理。
//!
//! 没传完的上传只有一条待完成记录（`core/files/pending/<编号>.json`），不算文件：`/_wonton/files/{id}` 查不到它。
//! 不读 tusd 的 `.info`，那是它的内部格式。
//!
//! tusd 的 pre-finish 钩子有 15 秒的硬超时，放不下跨卷复制，所以放文件走 post-finish；它不等钩子就回客户端，
//! 网关要在最后一个 PATCH 的响应之前等 [`Files::finish_upload`]，204 才等于文件可用（实测见 ADR-0020 的“验证结果”）。

mod abandon;
mod creating;
mod pending;
mod place;
#[cfg(test)]
mod private_tests;
mod room;
mod state;
mod sweep;
mod turns;

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub(super) use pending::{Pending, Placing};
pub(super) use room::has_room;
pub use state::{NewUpload, UploadState};
pub use turns::UploadTurn;
use wonton_protocol::{FileFailure, FileId};

use super::record::{FileRecord, Lookup, Origin};
use super::share_fs::{TRANSFERS, sanitize_name};
use super::{Failure, Files, OpenError};
use crate::l10n;
use crate::plugin::hostdir::{self, Owner};
use crate::plugin::languages::Languages;
use crate::plugin::manager::now_secs;
use crate::plugin::store::{create_private_dir, write_json};

/// 这么久没有进度的上传被清掉。tusd 没有实现 tus 的 expiration，只能由核心来清。
pub const STALE_SECS: u64 = 24 * 60 * 60;
/// 传完放好、链接还没发出去的上传留这么久。授权给房间或者发到设备时不再过期（ADR-0020 的“生命周期”）。
pub const UPLOAD_TTL_SECS: u64 = 24 * 60 * 60;
/// 放下上传后磁盘上至少还要空着这个比例。
const KEEP_FREE_PERCENT: u64 = 5;
/// 客户端走了的编号记这么久：tusd 的 pre-create 钩子晚到时要认得出来，钩子不会晚这么多。
const WITHDRAWN_KEEP_SECS: u64 = 60 * 60;

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
    creating: creating::Creating,
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
            creating: creating::Creating::default(),
            #[cfg(test)]
            abandon_stalled: tokio::sync::RwLock::default(),
        })
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
}

impl Files {
    /// 创建上传（tusd 的 pre-create）：查磁盘、发编号。没传完的编号查不到，所以这时不写编号记录。
    ///
    /// 放到单独的任务里做：tusd 等钩子有期限（`-hooks-http-timeout`，默认 15 秒），核心写待完成记录慢过它时会把钩子连接关掉，
    /// 这一层 async fn 的 future 跟着被丢掉。放进任务以后，登记（[`Self::upload_being_created`]）和这个上传的队活到真的做完，
    /// 刚做完的一小会儿里也还认：网关判得出是核心自己慢、而不是把 tusd 的裸 500 转给客户端；作废也排得在写记录后面，不会写完记录没人认。
    pub async fn create_upload(
        self: &Arc<Self>,
        upload: NewUpload<'_>,
        langs: &Languages,
    ) -> Result<FileId, Failure> {
        let files = self.clone();
        let langs = langs.clone();
        let (id, name, mime_type, size, device) = (
            upload.id.clone(),
            upload.name.map(str::to_owned),
            upload.mime_type.map(str::to_owned),
            upload.size,
            upload.device.to_owned(),
        );
        tokio::spawn(async move {
            let upload = NewUpload {
                id: &id,
                name: name.as_deref(),
                mime_type: mime_type.as_deref(),
                size,
                device: &device,
            };
            files.create_upload_in_turn(upload, &langs).await
        })
        .await
        .map_err(|e| Failure::internal(format!("建上传的任务中止：{e}")))?
    }

    /// 建上传本身：登记还在做，排上这个上传的队，查完磁盘、写待完成记录。
    async fn create_upload_in_turn(
        self: &Arc<Self>,
        upload: NewUpload<'_>,
        langs: &Languages,
    ) -> Result<FileId, Failure> {
        let invalid = |why: &str| Failure::new(FileFailure::InvalidInput, why);
        let _creating = self.uploads.creating.enter(upload.id);
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
        let (username, accessor) = self.host.upload_person(upload.device)?;
        let pending = Pending {
            id: upload.id.clone(),
            name: sanitize_name(name),
            mime_type: upload.mime_type.map(str::to_owned),
            size,
            device: upload.device.to_owned(),
            username,
            accessor,
            created_at: now_secs(),
            placing: None,
            abandoned: false,
        };
        let langs = langs.clone();
        self.blocking(move |files| {
            let _home = files
                .home_changes
                .read()
                .expect("home changes lock poisoned");
            files.host.check_person(&pending.accessor)?;
            let shares = files.home_shares(&pending.username, true)?;
            for dir in [
                files.uploads.dir.as_path(),
                shares.root().join(TRANSFERS).as_path(),
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
}

#[cfg(test)]
impl Files {
    /// 钩子问过核心之后的样子：只有待完成记录。磁盘放不放得下不查。
    pub(crate) fn pending_for_test(&self, id: &FileId, size: u64, device: &str) {
        let (username, accessor) = self.host.upload_person(device).unwrap();
        self.home_shares(&username, true).unwrap();
        let pending = Pending {
            id: id.clone(),
            name: "a.txt".to_owned(),
            mime_type: None,
            size,
            device: device.to_owned(),
            username,
            accessor,
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
