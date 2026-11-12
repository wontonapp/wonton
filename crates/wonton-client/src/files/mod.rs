//! 文件服务的客户端（ADR-0020）：传输列表、文件卡片、缩略图、给播放器的地址。网络操作都在核心自己的运行时里执行。
//!
//! 传输的结果分四种：做完；中断（[`api::Problem::Interrupted`]、[`api::Problem::ServicesDown`]），状态是暂停，连上、回到前台、过一会儿都会再试；失败，状态里写明原因，
//! 不再试；文件传完了链接没发出，等主人点重发。

mod api;
mod download;
mod opener;
mod playback;
mod store;
mod stream;
mod upload;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use tokio::runtime::Handle;
use tokio::sync::{Semaphore, watch};
use tokio::task::JoinHandle;
use wonton_protocol::{
    Action, CoreError, FileCard, FileFailure, FileId, FileInfo, FileOperation, FileProblem,
    HomeStatus, HomeViewModel, TransferDirection, TransferState, TransfersViewModel, UploadSource,
};

use crate::chat::ChatLink;
use crate::home::HomeLink;
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
    getrandom::fill(&mut bytes).expect("操作系统随机数源不可用");
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
            _ => Err(invalid("不是文件操作")),
        }
    }

    fn current_server(&self) -> Result<String, CoreError> {
        self.home
            .view()
            .server_id
            .ok_or_else(|| CoreError::NotReady {
                reason: "还没连接家里的服务器".to_owned(),
            })
    }

    fn start_upload(
        self: &Arc<Self>,
        source: UploadSource,
        name: String,
        mime_type: Option<String>,
        room: Option<String>,
    ) -> Result<(), CoreError> {
        if name.trim().is_empty() {
            return Err(invalid("文件名不能为空"));
        }
        let room = room.filter(|room| !room.is_empty());
        self.start(Record {
            id: local_id(),
            server: self.current_server()?,
            name,
            file: None,
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
            return Err(invalid("这个文件已经在下载了"));
        }
        let server = self.current_server()?;
        // 重下一次：之前那条做完或失败的说的已经不是这次了，留着的话不知道该看哪条。
        for old in same_file {
            self.store.remove(&old.id);
        }
        // 名字问到之前先用卡片上的，卡片没取过就显示编号。
        let name = lock(&self.cards)
            .get(&id)
            .map_or_else(|| id.to_string(), |(_, info)| info.name.clone());
        self.start(Record {
            id: local_id(),
            server,
            name,
            file: Some(id),
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

    /// 只重发链接。先改状态再开工：连点两下时第二下就不算了。
    fn resend_link(self: &Arc<Self>, id: &str) -> Result<(), CoreError> {
        let Some(record) = self.store.get(id) else {
            return Err(invalid("没有这条传输"));
        };
        if !matches!(record.state, TransferState::LinkNotSent { .. }) {
            return Err(invalid("这条传输没有等着重发的链接"));
        }
        self.store.update(id, |record| {
            record.state = TransferState::Running;
        });
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
        let (link, owned) = (self.clone(), id.to_owned());
        let worker = self.runtime.spawn(async move {
            link.work(&owned).await;
            lock(&link.workers).remove(&owned);
        });
        workers.insert(id.to_owned(), worker);
    }

    async fn work(self: &Arc<Self>, id: &str) {
        let Some(record) = self.store.update(id, |record| {
            record.state = TransferState::Running;
            record.paused_because = None;
        }) else {
            return;
        };
        let started_at = record.transferred_bytes;
        let current = self.home.view().server_id;
        let outcome = if current.as_deref() == Some(record.server.as_str()) {
            self.transfer(record).await
        } else {
            Err(Problem::Failed(
                FileFailure::FileUnknown,
                "换了家里的服务器，这个传输属于之前那一台".to_owned(),
            ))
        };
        let updated = self.store.update(id, |record| {
            record.state = match outcome {
                Ok(()) => TransferState::Done,
                Err(Problem::Interrupted(message)) => {
                    tracing::info!(transfer = %record.id, %message, "传输中断，等连上再接着传");
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
        let moved = updated.is_some_and(|record| {
            record.state == TransferState::Paused && record.transferred_bytes > started_at
        });
        if moved {
            let link = self.clone();
            self.runtime.spawn(async move {
                tokio::time::sleep(RETRY_SOON).await;
                link.foregrounded();
            });
        }
    }

    async fn transfer(self: &Arc<Self>, record: Record) -> Result<(), Problem> {
        let queue = match record.job {
            Job::Upload { .. } => &self.uploads,
            Job::Download { .. } => &self.downloads,
        };
        let _turn = queue.clone().acquire_owned().await.expect("信号量不会关闭");
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
            return Err(invalid("没有这条传输"));
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
            (Job::Download { path, .. }, _) => self.remove_part(path.as_deref()),
        }
        Ok(())
    }

    fn dismiss(&self, id: &str, delete_local_file: bool) -> Result<(), CoreError> {
        let Some(record) = self.store.get(id) else {
            return Err(invalid("没有这条传输"));
        };
        if matches!(record.state, TransferState::Running | TransferState::Paused) {
            return Err(invalid("这条传输还在进行，要先取消"));
        }
        let path = match &record.job {
            Job::Upload { .. } if delete_local_file => {
                return Err(invalid("上传的原文件不归 App 管"));
            }
            Job::Upload { .. } => None,
            Job::Download { path, .. } => path.as_deref(),
        };
        if delete_local_file && let Some(path) = path {
            self.delete_local_file(path)?;
        }
        self.remove_part(path);
        self.store.remove(id);
        Ok(())
    }

    /// 删不掉时不清记录，如实告诉主人：不然文件还占着地方，却没了入口。
    fn delete_local_file(&self, path: &str) -> Result<(), CoreError> {
        let path = self.downloads_dir.join(path);
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                return Err(CoreError::Storage {
                    reason: format!("删不掉手机上的文件：{e}"),
                });
            }
        }
        if let Some(parent) = path.parent()
            && let Err(e) = std::fs::remove_dir(parent)
        {
            tracing::debug!(error = %e, "下载目录没有清掉");
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
                tracing::info!(error = %e, "删不掉下载的半截文件");
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
                let what = match operation {
                    FileOperation::Delete => "删除",
                    FileOperation::SendToDevices => "发到设备",
                    FileOperation::GrantToRoom => "授权",
                    FileOperation::Play => "播放",
                };
                link.store.set_problem(Some(FileProblem {
                    operation,
                    file: id.clone(),
                    reason,
                    message: format!("{what}文件 {id} 失败：{message}"),
                }));
            }
        });
    }

    /// 消息下面的文件卡片。做成的缓存一会儿，失败的下次再问。
    pub(crate) async fn card(&self, id: FileId) -> FileCard {
        if let Some((at, info)) = lock(&self.cards).get(&id)
            && at.elapsed() < CARD_TTL
        {
            return FileCard::Ready { info: info.clone() };
        }
        let fetched = match Api::connect(&self.home) {
            Ok(api) => api.info::<FileInfo>(&id).await,
            Err(problem) => Err(problem),
        };
        match fetched {
            Ok(info) => {
                lock(&self.cards).insert(id, (Instant::now(), info.clone()));
                FileCard::Ready { info }
            }
            Err(problem) => {
                let (reason, message) = problem.into_parts();
                FileCard::Failed { reason, message }
            }
        }
    }

    pub(crate) async fn thumbnail(&self, id: FileId) -> Result<Vec<u8>, CoreError> {
        let api = Api::connect(&self.home).map_err(core_error)?;
        api.thumbnail(&id).await.map_err(core_error)
    }

    pub(crate) async fn stream_url(&self, id: FileId) -> Result<String, CoreError> {
        if self.home.view().proxy.is_none() {
            return Err(CoreError::TransportUnavailable {
                reason: "还没连上家里的服务器".to_owned(),
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
async fn follow_home(link: Weak<FilesLink>, mut home: watch::Receiver<HomeViewModel>) {
    loop {
        let changed = tokio::select! {
            changed = home.changed() => changed.is_ok(),
            () = tokio::time::sleep(RETRY) => true,
        };
        if !changed {
            return;
        }
        let connected = home.borrow_and_update().status == HomeStatus::Connected;
        let Some(link) = link.upgrade() else {
            return;
        };
        if connected {
            link.resume();
        } else if !link.store.paused().is_empty() {
            link.home.foregrounded();
        }
    }
}
