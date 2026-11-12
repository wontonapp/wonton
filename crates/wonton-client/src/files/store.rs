//! 传输列表：存在本机的 `transfers.json`，App 重启后接着传。状态变化时写盘，进度只进 ViewModel，真实的进度重启后问核心
//! （上传）或者看本机的半截文件（下载）。

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};
use tokio::sync::watch;
use wonton_l10n::l10n;
use wonton_protocol::{
    CoreError, FileId, FileProblem, FileSource, Transfer, TransferAction, TransferDirection,
    TransferState, TransfersViewModel, UploadSource,
};

use crate::home::write_private;
use crate::l10n::{CATALOG, language};

/// 上传完的留多少条：壳不显示，也就没人清。下载完的不自动清：那是主人手机上的文件，主人自己选清不清。
const KEEP_UPLOADED: usize = 50;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct Record {
    pub(super) id: String,
    /// 开始这次传输时的家里服务器：编号只在那一台上有意义。
    pub(super) server: String,
    pub(super) name: String,
    pub(super) file: Option<FileId>,
    /// 文件的来源，问到名字时记下：连不上家里的时候，手机上这份的卡片还要写明来源。
    #[serde(default)]
    pub(super) source: Option<FileSource>,
    pub(super) transferred_bytes: u64,
    pub(super) total_bytes: Option<u64>,
    pub(super) state: TransferState,
    /// 不写盘：App 重启后再试一次就有了。
    #[serde(skip)]
    pub(super) paused_because: Option<String>,
    pub(super) job: Job,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum Job {
    Upload {
        source: UploadSource,
        mime_type: Option<String>,
        room: Option<String>,
        /// 第一次打开原文件时的样子：接着传时对不上，就不是同一个文件了。
        stamp: Option<SourceStamp>,
        /// 字节传完了，只差发链接。
        #[serde(default)]
        uploaded: bool,
    },
    Download {
        /// 相对下载目录的路径，问到文件名后才有。不存绝对路径：iOS 上 App 更新后容器的路径会变。
        path: Option<String>,
        /// 半截文件对应的 ETag：接着下时交给 `If-Range`，文件变了核心给整个文件。
        etag: Option<String>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct SourceStamp {
    pub(super) size: u64,
    /// 不用 i128：serde 的带标签枚举读不了。
    pub(super) modified_ns: u64,
}

impl Record {
    /// 下载的内容对应的 ETag：接着下、核对新旧都用它。
    pub(super) fn etag(&self) -> Option<String> {
        match &self.job {
            Job::Download { etag, .. } => etag.clone(),
            Job::Upload { .. } => None,
        }
    }
}

pub(super) struct Store {
    path: PathBuf,
    downloads_dir: PathBuf,
    records: Mutex<Vec<Record>>,
    view: watch::Sender<TransfersViewModel>,
    /// 核心在释放：还在跑的传输会读到“后台任务失败”这样的假错误，不能记下来，重启后要接着传。
    closed: AtomicBool,
}

impl Store {
    /// 读不懂的列表不拦着 App 启动：挪到一边、记下错误，从空列表开始。App 被杀时还在传的算暂停。
    pub(super) fn open(
        path: PathBuf,
        downloads_dir: PathBuf,
        view: Option<watch::Sender<TransfersViewModel>>,
    ) -> Self {
        let mut records = read(&path);
        for record in &mut records {
            if record.state == TransferState::Running {
                record.state = TransferState::Paused;
            }
        }
        let view = crate::scope::reuse(view, view_of(&records, &downloads_dir, None));
        Self {
            path,
            downloads_dir,
            records: Mutex::new(records),
            view,
            closed: AtomicBool::new(false),
        }
    }

    pub(super) fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Record>> {
        self.records
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(super) fn view(&self) -> TransfersViewModel {
        self.view.borrow().clone()
    }

    pub(super) fn sender(&self) -> watch::Sender<TransfersViewModel> {
        self.view.clone()
    }

    /// 写盘失败时不加：不然主人以为 App 重启后还能接着传。
    pub(super) fn insert(&self, record: Record) -> Result<(), CoreError> {
        let mut records = self.lock();
        let mut updated = records.clone();
        updated.insert(0, record);
        prune(&mut updated);
        self.write(&updated)
            .map_err(|reason| CoreError::Storage { reason })?;
        *records = updated;
        self.publish(&records);
        Ok(())
    }

    pub(super) fn get(&self, id: &str) -> Option<Record> {
        self.lock().iter().find(|record| record.id == id).cloned()
    }

    /// 手机上有完整文件的下载记录，最新的那条：接着下、核对新旧、离线给卡片都用它。
    pub(super) fn finished_download(&self, file: &FileId) -> Option<Record> {
        self.lock()
            .iter()
            .find(|record| {
                record.state == TransferState::Done
                    && matches!(record.job, Job::Download { .. })
                    && record.file.as_ref() == Some(file)
            })
            .cloned()
    }

    pub(super) fn paused(&self) -> Vec<String> {
        let records = self.lock();
        let paused = records.iter().filter(|r| r.state == TransferState::Paused);
        paused.map(|record| record.id.clone()).collect()
    }

    /// 改一条并写盘，返回改完的样子；已经被清掉时为空。写盘失败不停传输：这次传完不受影响，只是 App 被杀后接不上，记下错误。
    pub(super) fn update(&self, id: &str, change: impl FnOnce(&mut Record)) -> Option<Record> {
        if self.closed.load(Ordering::SeqCst) {
            return None;
        }
        let mut records = self.lock();
        let record = records.iter_mut().find(|record| record.id == id)?;
        change(record);
        let updated = record.clone();
        if let Err(error) = self.write(&records) {
            tracing::error!(error, "could not write the transfer list");
        }
        self.publish(&records);
        Some(updated)
    }

    /// 进度：只进 ViewModel，不写盘。
    pub(super) fn progress(&self, id: &str, transferred: u64) {
        let mut records = self.lock();
        if let Some(record) = records.iter_mut().find(|record| record.id == id) {
            record.transferred_bytes = transferred;
            self.publish(&records);
        }
    }

    pub(super) fn remove(&self, id: &str) -> Option<Record> {
        let mut records = self.lock();
        let index = records.iter().position(|record| record.id == id)?;
        let removed = records.remove(index);
        if let Err(error) = self.write(&records) {
            tracing::error!(error, "could not write the transfer list");
        }
        self.publish(&records);
        Some(removed)
    }

    pub(super) fn set_problem(&self, problem: Option<FileProblem>) {
        self.view.send_modify(|view| view.problem = problem);
    }

    fn write(&self, records: &[Record]) -> Result<(), String> {
        let bytes = serde_json::to_vec(records).expect("transfer records always serialize");
        write_private(&self.path, &bytes).map_err(|e| {
            l10n!(
                CATALOG,
                language(),
                "files-store-write-failed",
                path = self.path.display().to_string(),
                reason = e.to_string()
            )
        })
    }

    fn publish(&self, records: &[Record]) {
        self.view.send_modify(|view| {
            *view = view_of(records, &self.downloads_dir, view.problem.take());
        });
    }
}

fn read(path: &PathBuf) -> Vec<Record> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(e) => {
            tracing::error!(error = %e, "could not read the transfer list; starting from an empty list");
            return Vec::new();
        }
    };
    serde_json::from_slice(&bytes).unwrap_or_else(|e| {
        tracing::error!(error = %e, "the transfer list is unreadable; moving it to .bad and starting from an empty list");
        if let Err(e) = fs::rename(path, path.with_extension("bad")) {
            tracing::error!(error = %e, "could not move the unreadable transfer list aside");
        }
        Vec::new()
    })
}

/// 列表从新到旧。旧的先走。
fn prune(records: &mut Vec<Record>) {
    let mut uploaded = 0;
    records.retain(|record| {
        if record.state != TransferState::Done || !matches!(record.job, Job::Upload { .. }) {
            return true;
        }
        uploaded += 1;
        uploaded <= KEEP_UPLOADED
    });
}

fn view_of(
    records: &[Record],
    downloads_dir: &Path,
    problem: Option<FileProblem>,
) -> TransfersViewModel {
    let mut view = TransfersViewModel {
        problem,
        ..TransfersViewModel::default()
    };
    for record in records {
        let transfer = transfer(record, downloads_dir);
        let list = match (&transfer.state, transfer.direction) {
            (TransferState::Done, TransferDirection::Download) => &mut view.downloaded,
            (TransferState::Done, TransferDirection::Upload) => &mut view.uploaded,
            _ => &mut view.active,
        };
        list.push(transfer);
    }
    view
}

fn transfer(record: &Record, downloads_dir: &Path) -> Transfer {
    let (direction, room, destination) = match &record.job {
        Job::Upload { room, .. } => (TransferDirection::Upload, room.clone(), None),
        Job::Download { path, .. } => (
            TransferDirection::Download,
            None,
            path.as_ref()
                .map(|path| downloads_dir.join(path).to_string_lossy().into_owned()),
        ),
    };
    Transfer {
        id: record.id.clone(),
        file: record.file.clone(),
        name: record.name.clone(),
        direction,
        room,
        transferred_bytes: record.transferred_bytes,
        total_bytes: record.total_bytes,
        actions: actions(record),
        state: record.state.clone(),
        paused_because: record.paused_because.clone(),
        destination,
    }
}

fn actions(record: &Record) -> Vec<TransferAction> {
    let uploaded = matches!(record.job, Job::Upload { uploaded: true, .. });
    let download = matches!(record.job, Job::Download { .. });
    match &record.state {
        TransferState::Running | TransferState::Paused if uploaded => {
            vec![TransferAction::StopSending]
        }
        TransferState::Running | TransferState::Paused => vec![TransferAction::Cancel],
        TransferState::Done if download => vec![
            TransferAction::Open,
            TransferAction::Share,
            TransferAction::DismissDownloaded,
        ],
        TransferState::LinkNotSent { .. } => {
            vec![TransferAction::ResendLink, TransferAction::StopSending]
        }
        TransferState::Done | TransferState::Failed { .. } => vec![TransferAction::Dismiss],
    }
}
