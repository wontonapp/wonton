//! 文件服务的客户端（ADR-0020）：传输列表、文件卡片、缩略图、给播放器的地址。网络操作都在核心自己的运行时里执行。
//!
//! 传输的结果分四种：做完；中断（[`api::Problem::Interrupted`]、[`api::Problem::ServicesDown`]），状态是暂停，连上、回到前台、过一会儿都会再试；失败，状态里写明原因，
//! 不再试；文件传完了链接没发出，等主人点重发。

mod api;
mod download;
mod held;
mod opener;
mod playback;
mod store;
mod stream;
mod upload;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use tokio::runtime::Handle;
use tokio::sync::{Semaphore, watch};
use tokio::task::JoinHandle;
use wonton_l10n::l10n;
use wonton_protocol::{
    Action, CoreError, FileCard, FileFailure, FileId, FileInfo, FileOperation, FileProblem,
    FileSource, HomeStatus, HomeViewModel, TransferDirection, TransferState, TransfersViewModel,
    UploadSource,
};

use crate::chat::ChatLink;
use crate::home::HomeLink;
use crate::l10n::{CATALOG, language};
use api::{Api, Problem};
use store::{Job, Record, Store};
use stream::StreamServer;

pub use opener::{FileOpenError, FileOpener};

/// 同时传几个：手机的上行有限，一起传谁也传不快。多的排队，状态也是传输中。
const MAX_PER_DIRECTION: usize = 2;
/// 暂停的传输多久再试一次：隧道的状态有时看不出某条流断了。
const RETRY: Duration = Duration::from_secs(30);
/// 这次传动了一些才断的话很快再试：多半只是这条连接断了。一点没动的等 [`RETRY`]，不然核心一直不回答时会不停地重试。
const RETRY_SOON: Duration = Duration::from_secs(3);
/// 文件卡片缓存多久。来源和大小几乎不变，滚动时间线不用每次问。失败的不缓存。
const CARD_TTL: Duration = Duration::from_secs(60);

pub(crate) struct FilesLink {
    runtime: Handle,
    home: Arc<HomeLink>,
    chat: Arc<ChatLink>,
    store: Store,
    downloads_dir: PathBuf,
    opener: Mutex<Option<Arc<dyn FileOpener>>>,
    /// 在跑的传输。
    workers: Mutex<HashMap<String, JoinHandle<()>>>,
    uploads: Arc<Semaphore>,
    downloads: Arc<Semaphore>,
    cards: Mutex<HashMap<FileId, (Instant, FileInfo)>>,
    /// 卡片问过核心、服务器上的内容和手机上这份对不上的编号：卡片不再让直接打开，给重新下载。
    stale: Mutex<HashSet<FileId>>,
    /// 上次对着服务器核对手机这份是什么时候：卡片常被重新取，核对最多隔 [`CARD_TTL`] 一次。
    checked: Mutex<HashMap<FileId, Instant>>,
    stream: StreamServer,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn invalid(reason: &str) -> CoreError {
    CoreError::InvalidInput {
        reason: reason.to_owned(),
    }
}

fn local_id() -> String {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).expect("the OS random source is unavailable");
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

impl FilesLink {
    pub(crate) fn new(
        runtime: Handle,
        home: Arc<HomeLink>,
        chat: Arc<ChatLink>,
        data_dir: &Path,
        view: Option<watch::Sender<TransfersViewModel>>,
    ) -> Arc<Self> {
        let link = Arc::new(Self {
            runtime: runtime.clone(),
            store: Store::open(
                data_dir.join("transfers.json"),
                data_dir.join("downloads"),
                view,
            ),
            downloads_dir: data_dir.join("downloads"),
            home: home.clone(),
            chat,
            opener: Mutex::new(None),
            workers: Mutex::new(HashMap::new()),
            uploads: Arc::new(Semaphore::new(MAX_PER_DIRECTION)),
            downloads: Arc::new(Semaphore::new(MAX_PER_DIRECTION)),
            cards: Mutex::new(HashMap::new()),
            stale: Mutex::new(HashSet::new()),
            checked: Mutex::new(HashMap::new()),
            stream: StreamServer::new(),
        });
        runtime.spawn(follow_home(Arc::downgrade(&link), home.subscribe()));
        link
    }

    /// 核心释放前调用，运行时关掉之前。
    pub(crate) fn close(&self) {
        self.store.close();
    }

    pub(crate) fn set_opener(&self, opener: Arc<dyn FileOpener>) {
        *lock(&self.opener) = Some(opener);
    }

    fn opener(&self) -> Option<Arc<dyn FileOpener>> {
        lock(&self.opener).clone()
    }

    pub(crate) fn view(&self) -> TransfersViewModel {
        self.store.view()
    }

    /// 换服务器时新作用域接着用这个发送端，壳的订阅不断（ADR-0042）。
    pub(crate) fn sender(&self) -> watch::Sender<TransfersViewModel> {
        self.store.sender()
    }

    pub(crate) fn foregrounded(self: &Arc<Self>) {
        if self.home.view().status == HomeStatus::Connected {
            self.resume();
        }
    }

    pub(crate) fn dispatch(self: &Arc<Self>, action: Action) -> Result<(), CoreError> {
        match action {
            Action::UploadFile {
                source,
                name,
                mime_type,
                room,
            } => self.start_upload(source, name, mime_type, room),
            Action::DownloadFile { id } => self.start_download(id),
            Action::CancelTransfer { id } => self.cancel(&id),
            Action::DismissTransfer {
                id,
                delete_local_file,
            } => self.dismiss(&id, delete_local_file),
            Action::ResendFileLink { id } => self.resend_link(&id),
            Action::DeleteFile { id } => {
                lock(&self.cards).remove(&id);
                lock(&self.stale).remove(&id);
                lock(&self.checked).remove(&id);
                self.operate(FileOperation::Delete, id, |api, id| async move {
                    api.delete(&id).await
                });
                Ok(())
            }
            Action::SendFileToDevices { id } => {
                self.operate(FileOperation::SendToDevices, id, |api, id| async move {
                    api.send_to_devices(&id).await
                });
                Ok(())
            }
            Action::GrantFileToRoom { id, room } => {
                self.operate(FileOperation::GrantToRoom, id, |api, id| async move {
                    api.grant(&id, &room).await
                });
                Ok(())
            }
            Action::DismissFileProblem => {
                self.store.set_problem(None);
                Ok(())
            }
            _ => Err(invalid(&l10n!(CATALOG, language(), "files-invalid-action"))),
        }
    }

    fn current_server(&self) -> Result<String, CoreError> {
        self.home
            .view()
            .server_id
            .ok_or_else(|| CoreError::NotReady {
                reason: l10n!(CATALOG, language(), "files-api-not-connected"),
            })
    }

    fn start_upload(
        self: &Arc<Self>,
        source: UploadSource,
        name: String,
        mime_type: Option<String>,
        room: Option<String>,
    ) -> Result<(), CoreError> {
        let room = room.filter(|room| !room.is_empty());
        let record = self.upload_record(source, name, mime_type, room)?;
        self.start(record)
    }

    fn upload_record(
        &self,
        source: UploadSource,
        name: String,
        mime_type: Option<String>,
        room: Option<String>,
    ) -> Result<Record, CoreError> {
        if name.trim().is_empty() {
            return Err(invalid(&l10n!(CATALOG, language(), "files-name-required")));
        }
        Ok(Record {
            id: local_id(),
            server: self.current_server()?,
            name,
            file: None,
            source: None,
            transferred_bytes: 0,
            total_bytes: None,
            state: TransferState::Running,
            paused_because: None,
            job: Job::Upload {
                source,
                mime_type,
                room,
                stamp: None,
                uploaded: false,
            },
        })
    }

    fn start_download(self: &Arc<Self>, id: FileId) -> Result<(), CoreError> {
        let same_file: Vec<_> = self
            .store
            .view()
            .all()
            .filter(|transfer| {
                transfer.direction == TransferDirection::Download
                    && transfer.file.as_ref() == Some(&id)
            })
            .cloned()
            .collect();
        let writing = same_file.iter().any(|transfer| {
            matches!(
                transfer.state,
                TransferState::Running | TransferState::Paused
            )
        });
        if writing {
            return Err(invalid(&l10n!(
                CATALOG,
                language(),
                "files-download-already-running"
            )));
        }
        let server = self.current_server()?;
        // 重下一次：之前那条做完或失败的说的已经不是这次了，留着的话不知道该看哪条。
        for old in same_file {
            self.store.remove(&old.id);
        }
        // 旧名字的文件没有入口（服务器上的文件改过名），里面没人认领的东西一起清掉；半截的也从头来。
        self.remove_download_dir(&id)?;
        lock(&self.stale).remove(&id);
        lock(&self.checked).remove(&id);
        // 名字问到之前先用卡片上的，卡片没取过就显示编号。
        let cached = lock(&self.cards).get(&id).map(|(_, info)| info.clone());
        let name = cached
            .as_ref()
            .map_or_else(|| id.to_string(), |info| info.name.clone());
        self.start(Record {
            id: local_id(),
            server,
            name,
            file: Some(id),
            source: cached.map(|info| info.source),
            transferred_bytes: 0,
            total_bytes: None,
            state: TransferState::Running,
            paused_because: None,
            job: Job::Download {
                path: None,
                etag: None,
            },
        })
    }

    /// 只重发链接。不欠链接时什么都不做：连点两下第二下不算错（壳不弹窗），也不重跑一遍；做完的、失败的
    /// 更不该再跑，带房间的上传会多发一条链接。
    fn resend_link(self: &Arc<Self>, id: &str) -> Result<(), CoreError> {
        if self.store.get(id).is_none() {
            return Err(invalid(&l10n!(
                CATALOG,
                language(),
                "files-transfer-not-found"
            )));
        }
        self.spawn_worker(id);
        Ok(())
    }

    fn start(self: &Arc<Self>, record: Record) -> Result<(), CoreError> {
        let id = record.id.clone();
        self.store.insert(record)?;
        self.spawn_worker(&id);
        Ok(())
    }

    /// 暂停的都接着传。
    fn resume(self: &Arc<Self>) {
        for id in self.store.paused() {
            self.spawn_worker(&id);
        }
    }

    fn spawn_worker(self: &Arc<Self>, id: &str) {
        let mut workers = lock(&self.workers);
        if workers.contains_key(id) {
            return;
        }
        // 登记和接手是一步（同一把锁），收尾的 worker 把终态写进同一个锁才注销：撞上收尾的接手在锁外等一拍，
        // 接手时状态已经落定，不会吞掉一拍重发或恢复。不看原状态就改成进行中的话，两次撞上会把做完的改回去
        // 再跑一遍（带房间的上传多发一条链接）。
        let Some(record) = self.store.update(id, |record| {
            if matches!(
                record.state,
                TransferState::Running | TransferState::Paused | TransferState::LinkNotSent { .. }
            ) {
                record.state = TransferState::Running;
                record.paused_because = None;
            }
        }) else {
            return;
        };
        if !matches!(record.state, TransferState::Running) {
            return;
        }
        let (link, owned) = (self.clone(), id.to_owned());
        let worker = self.runtime.spawn(async move {
            let Some((outcome, started_at)) = link.work(&owned).await else {
                lock(&link.workers).remove(&owned);
                return;
            };
            // 终态的写入和注销是一步（同一把锁）：重发、恢复撞上收尾时，在锁外等一拍，接手时状态已经落定，
            // 不会有一拍被吞掉、也不会接手到还没写完的中间状态。
            let mut workers = lock(&link.workers);
            let updated = link.store.update(&owned, |record| {
                record.state = match outcome {
                    Ok(()) => TransferState::Done,
                    Err(Problem::Interrupted(message)) => {
                        tracing::info!(transfer = %record.id, %message, "transfer interrupted; it will resume once connected");
                        TransferState::Paused
                    }
                    Err(Problem::ServicesDown(message)) => {
                        record.paused_because = Some(message);
                        TransferState::Paused
                    }
                    Err(Problem::Failed(reason, message)) => TransferState::Failed { reason, message },
                    Err(Problem::LinkNotSent(reason, message)) => {
                        TransferState::LinkNotSent { reason, message }
                    }
                };
            });
            workers.remove(&owned);
            if updated.is_some_and(|record| {
                record.state == TransferState::Paused && record.transferred_bytes > started_at
            }) {
                drop(workers);
                let kick = link.clone();
                link.runtime.spawn(async move {
                    tokio::time::sleep(RETRY_SOON).await;
                    kick.foregrounded();
                });
            }
        });
        workers.insert(id.to_owned(), worker);
    }

    async fn work(self: &Arc<Self>, id: &str) -> Option<(Result<(), Problem>, u64)> {
        // 接手时状态已经是进行中了（spawn_worker 在登记的同时改写），这里只读。
        let record = self.store.get(id)?;
        let started_at = record.transferred_bytes;
        let current = self.home.view().server_id;
        let outcome = if current.as_deref() == Some(record.server.as_str()) {
            self.transfer(record).await
        } else {
            Err(Problem::Failed(
                FileFailure::FileUnknown,
                l10n!(CATALOG, language(), "files-transfer-server-changed"),
            ))
        };
        Some((outcome, started_at))
    }

    async fn transfer(self: &Arc<Self>, record: Record) -> Result<(), Problem> {
        let queue = match record.job {
            Job::Upload { .. } => &self.uploads,
            Job::Download { .. } => &self.downloads,
        };
        let _turn = queue
            .clone()
            .acquire_owned()
            .await
            .expect("the semaphore is never closed");
        match record.job {
            Job::Upload { .. } => upload::run(self, record).await,
            Job::Download { .. } => download::run(self, record).await,
        }
    }

    fn stop_worker(&self, id: &str) {
        if let Some(worker) = lock(&self.workers).remove(id) {
            worker.abort();
        }
    }

    /// 取消和传完撞上时不算错：已经做完或失败的不动。字节传完后只是不发链接了：tus 不再认这个编号，文件留在服务器上，由核心到期清掉
    /// （ADR-0020）。
    fn cancel(self: &Arc<Self>, id: &str) -> Result<(), CoreError> {
        let Some(record) = self.store.get(id) else {
            return Err(invalid(&l10n!(
                CATALOG,
                language(),
                "files-transfer-not-found"
            )));
        };
        if !matches!(
            record.state,
            TransferState::Running | TransferState::Paused | TransferState::LinkNotSent { .. }
        ) {
            return Ok(());
        }
        self.stop_worker(id);
        let Some(record) = self.store.remove(id) else {
            return Ok(());
        };
        match (record.job, record.file) {
            (Job::Upload { uploaded: true, .. }, _) => {}
            (Job::Upload { .. }, Some(file)) => {
                if let Ok(api) = Api::connect(&self.home) {
                    self.runtime.spawn(upload::abort(api, file));
                }
            }
            (Job::Upload { .. }, None) => {}
            // 下载的东西都在 downloads/<编号>/ 里：半截的、改名前留下的，连目录一起删。
            (Job::Download { .. }, Some(file)) => {
                if let Err(error) = self.remove_download_dir(&file) {
                    tracing::info!(error = %error, "could not remove the download folder");
                }
            }
            (Job::Download { .. }, None) => {}
        }
        Ok(())
    }

    fn dismiss(&self, id: &str, delete_local_file: bool) -> Result<(), CoreError> {
        let Some(record) = self.store.get(id) else {
            return Err(invalid(&l10n!(
                CATALOG,
                language(),
                "files-transfer-not-found"
            )));
        };
        if matches!(record.state, TransferState::Running | TransferState::Paused) {
            return Err(invalid(&l10n!(
                CATALOG,
                language(),
                "files-dismiss-transfer-running"
            )));
        }
        match &record.job {
            Job::Upload { .. } if delete_local_file => {
                return Err(invalid(&l10n!(
                    CATALOG,
                    language(),
                    "files-dismiss-upload-source-protected"
                )));
            }
            Job::Upload { .. } => {}
            // 带删除的清理连目录一起删：里面可能有改名前留下的、没人认领的文件。
            Job::Download { .. } if delete_local_file => {
                if let Some(file) = &record.file {
                    self.remove_download_dir(file)?;
                }
            }
            // 只清记录：文件留着，半截的删掉。
            Job::Download { path, .. } => self.remove_part(path.as_deref()),
        }
        self.store.remove(id);
        Ok(())
    }

    /// 下载的东西都放在 `downloads/<编号>/` 里：半截的、做完的、改名前留下的。连目录一起删，里面不留没人认领的
    /// 文件。删不掉时如实报错：接着下会把旧内容和新拼在一起。删不掉也不要紧的调用方自己记日志。
    /// 该是目录的地方是别人放的一个文件时不算我们的没删掉：里面没有我们的东西，下载接着走，建目录时如实报错。
    fn remove_download_dir(&self, id: &FileId) -> Result<(), CoreError> {
        let dir = self.downloads_dir.join(id.as_str());
        match std::fs::remove_dir_all(&dir) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotADirectory => {}
            Err(e) => {
                return Err(CoreError::Storage {
                    reason: l10n!(
                        CATALOG,
                        language(),
                        "files-delete-local-failed",
                        reason = e.to_string()
                    ),
                });
            }
        }
        Ok(())
    }

    fn remove_part(&self, path: Option<&str>) {
        let Some(path) = path else {
            return;
        };
        let part = download::part_path(&self.downloads_dir.join(path));
        self.runtime.spawn(async move {
            if let Err(e) = tokio::fs::remove_file(&part).await
                && e.kind() != std::io::ErrorKind::NotFound
            {
                tracing::info!(error = %e, "could not remove the partial download file");
            }
        });
    }

    /// 在后台做一个不落在某条传输上的操作，失败时把原因放进列表。这种操作没有进度，断了不重试：主人看到说明后自己再点一次。
    fn operate<F, Fut>(self: &Arc<Self>, operation: FileOperation, id: FileId, op: F)
    where
        F: FnOnce(Api, FileId) -> Fut + Send + 'static,
        Fut: Future<Output = Result<(), Problem>> + Send,
    {
        let link = self.clone();
        self.runtime.spawn(async move {
            let done = match Api::connect(&link.home) {
                Ok(api) => op(api, id.clone()).await,
                Err(problem) => Err(problem),
            };
            if let Err(problem) = done {
                let (reason, message) = problem.into_parts();
                let failed = match operation {
                    FileOperation::Delete => "files-delete-failed",
                    FileOperation::SendToDevices => "files-send-failed",
                    FileOperation::GrantToRoom => "files-grant-failed",
                    FileOperation::Play => "files-play-failed",
                };
                link.store.set_problem(Some(FileProblem {
                    operation,
                    file: id.clone(),
                    reason,
                    message: l10n!(
                        CATALOG,
                        language(),
                        failed,
                        id = id.to_string(),
                        message = message
                    ),
                }));
            }
        });
    }

    /// 消息下面的文件卡片。做成的缓存一会儿，失败的下次再问。连不上家里的服务器时，手机上有这份文件就照常给：
    /// 打开、分享都用不上网。
    pub(crate) async fn card(&self, id: FileId) -> FileCard {
        // 卡片常被重新取：顺路核对一次手机上这份新不新，最多隔 [`CARD_TTL`] 一轮。
        self.check_local_copy(&id).await;
        if let Some((at, info)) = lock(&self.cards).get(&id)
            && at.elapsed() < CARD_TTL
        {
            return FileCard::Ready { info: info.clone() };
        }
        let failed = |problem: Problem| {
            let (reason, message) = problem.into_parts();
            FileCard::Failed { reason, message }
        };
        match Api::connect(&self.home) {
            Ok(api) => match api.info::<FileInfo>(&id).await {
                Ok(info) => {
                    lock(&self.cards).insert(id, (Instant::now(), info.clone()));
                    FileCard::Ready { info }
                }
                Err(problem) => self.local_card(&id).unwrap_or_else(|| failed(problem)),
            },
            Err(problem) => self.local_card(&id).unwrap_or_else(|| failed(problem)),
        }
    }

    /// 手机上有完整文件时给一张不依赖连接的卡片：名字和大小在下载记录里，类型问不到就空着，卡片按扩展名给按钮。
    /// 记录清掉了（只清记录）文件也还在：下载目录里这个编号就一个文件。
    fn local_card(&self, id: &FileId) -> Option<FileCard> {
        match self.store.finished_download(id) {
            Some(record) => {
                let path = self
                    .downloads_dir
                    .join(download::relative_path(id, &record.name));
                path.is_file().then(|| FileCard::Ready {
                    info: FileInfo {
                        id: id.clone(),
                        name: record.name,
                        mime_type: None,
                        size: record.total_bytes,
                        source: record.source.unwrap_or(FileSource::Unknown),
                        has_thumbnail: false,
                    },
                })
            }
            None => {
                let mut files = std::fs::read_dir(self.downloads_dir.join(id.as_str()))
                    .ok()?
                    .filter_map(|entry| entry.ok())
                    .filter(|entry| entry.file_type().ok().is_some_and(|kind| kind.is_file()))
                    .collect::<Vec<_>>();
                files.sort_by_key(|entry| entry.file_name());
                let file = files.first()?;
                let name = file.file_name().to_string_lossy().into_owned();
                let size = file.metadata().ok().map(|meta| meta.len());
                Some(FileCard::Ready {
                    info: FileInfo {
                        id: id.clone(),
                        name,
                        mime_type: None,
                        size,
                        source: FileSource::Unknown,
                        has_thumbnail: false,
                    },
                })
            }
        }
    }

    /// 服务器上的文件改过后手机上这份就是旧的：记下来，卡片不再让直接打开，给重新下载。
    /// 只核对做完的下载：半截文件接着下时 `If-Range` 对不上会从头来，不用管。连不上就不妄说。
    async fn check_local_copy(&self, id: &FileId) {
        let Some(record) = self.store.finished_download(id) else {
            return;
        };
        let due = match lock(&self.checked).get(id) {
            Some(at) => at.elapsed() >= CARD_TTL,
            None => true,
        };
        if !due {
            return;
        }
        let path = self
            .downloads_dir
            .join(download::relative_path(id, &record.name));
        let (Some(saved), true) = (record.etag(), path.is_file()) else {
            // 没有 ETag 就对不上，文件不在了也没得核对：不妄说旧的。
            return;
        };
        lock(&self.checked).insert(id.clone(), Instant::now());
        let Ok(api) = Api::connect(&self.home) else {
            return;
        };
        let Some(etag) = api.content_etag(id).await else {
            // 核心没答上来：不妄说。
            return;
        };
        if etag == saved {
            lock(&self.stale).remove(id);
        } else {
            tracing::info!(file = %id, "the file at home has changed since it was downloaded");
            lock(&self.stale).insert(id.clone());
        }
    }

    pub(crate) async fn thumbnail(&self, id: FileId) -> Result<Vec<u8>, CoreError> {
        let api = Api::connect(&self.home).map_err(core_error)?;
        api.thumbnail(&id).await.map_err(core_error)
    }

    pub(crate) async fn stream_url(&self, id: FileId) -> Result<String, CoreError> {
        if self.home.view().proxy.is_none() {
            return Err(CoreError::TransportUnavailable {
                reason: l10n!(CATALOG, language(), "files-api-not-connected"),
            });
        }
        self.stream.url(&self.home, &id).await
    }
}

fn core_error(problem: Problem) -> CoreError {
    let (kind, reason) = problem.into_parts();
    match kind {
        FileFailure::TransportUnavailable => CoreError::TransportUnavailable { reason },
        FileFailure::Unauthenticated | FileFailure::NotShared | FileFailure::PermissionDenied => {
            CoreError::Unauthorized { reason }
        }
        FileFailure::FileUnknown
        | FileFailure::FileExpired
        | FileFailure::FileGone
        | FileFailure::NotViewable
        | FileFailure::InvalidInput => CoreError::InvalidInput { reason },
        _ => CoreError::Upstream { reason },
    }
}

/// 跟着家里的连接：连上了就接着传。还有暂停的传输而隧道断着时，隔一会儿主动检查连接：本地代理的连接断了要有人用才会重连。
/// 先看现在的状态再等变化：订阅之前就连上的话（换了服务器、重开了作用域），只等变化要多等一轮。
async fn follow_home(link: Weak<FilesLink>, mut home: watch::Receiver<HomeViewModel>) {
    loop {
        let connected = home.borrow_and_update().status == HomeStatus::Connected;
        let Some(link) = link.upgrade() else {
            return;
        };
        if connected {
            link.resume();
        } else if !link.store.paused().is_empty() {
            link.home.foregrounded();
        }
        let changed = tokio::select! {
            changed = home.changed() => changed.is_ok(),
            () = tokio::time::sleep(RETRY) => true,
        };
        if !changed {
            return;
        }
    }
}
