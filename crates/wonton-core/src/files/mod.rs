//! 文件服务（ADR-0020）：每个文件一个不可猜的编号，指向共享文件夹里的原文件、插件的资源或设备上传的文件，都不复制。
//!
//! 这里只管编号、访问范围和字节从哪来；HTTP 接口在 `gateway::files`，Bot 的工具在 MCP 总线上（登记和核对在 [`bot`]），都只经这里
//! 碰文件。
//!
//! | 访问者 | 不在范围里 | 编号过期 |
//! | --- | --- | --- |
//! | 主人（已配对的设备） | `not_shared` | `file_unknown` |
//! | Bot | `file_unknown` | `file_expired` |

mod bot;
pub mod content;
mod host;
pub mod record;
pub mod resource;
mod saving;
pub mod share_fs;
mod slots;
pub mod uploads;

use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use hyper::Method;
use iroh::EndpointId;
use serde::Serialize;
use tokio::sync::OwnedSemaphorePermit;
use wonton_protocol::{FileFailure, FileId, FileInfo};

pub use host::{CoreHost, FileHost};

use crate::mcp::audit::Audit;
use crate::plugin::PluginId;
use crate::plugin::hostdir::{self, DirError, Owner};
use crate::plugin::image_worker::{ImageError, ImageWorker};
use crate::plugin::images::MAX_THUMBNAIL_INPUT;
use crate::plugin::manager::now_secs;
use crate::plugin::store::StoreError;
use content::{can_thumbnail, declared_type, served_type};
use record::{Accessor, FileRecord, Lookup, Origin, Registry, RegistryError};
use resource::{InternalUrl, RemoteRequest};
use share_fs::{Deleted, Opened, ShareError, ShareRef, Shares, TRANSFERS};
use slots::{PluginSlot, PluginSlots};

/// `resources/read` 最多读这么多（插件规范）。
pub const MAX_RESOURCE_BYTES: usize = 32 << 20;
/// `wonton__view_file` 的文本分支最多读这么多字节：只看开头，不把整个文件读进内存。
pub const MAX_TEXT_PREVIEW_BYTES: usize = 64 << 10;
/// 拿到名额以后，读做缩略图的原文件最多这么久。
const THUMBNAIL_READ_TIMEOUT: Duration = Duration::from_secs(90);
// 里面的 `resources/read` 要先超时，那次超时才算到插件头上（`Upstreams::timed_out`）；它前面等名额的时间有自己的期限和错误。
const _: () = assert!(
    crate::mcp::upstream::READ_BUDGET.as_secs() + slots::SLOT_WAIT.as_secs()
        < THUMBNAIL_READ_TIMEOUT.as_secs(),
    "缩略图的期限要比等名额加读资源的长"
);
/// 同时最多读这么多张本地的原图，每张最多 [`MAX_THUMBNAIL_INPUT`]。插件的原图另有名额（[`slots`]）：存储卡住时插件的缩略图
/// 照常，插件慢时本地的也照常。和 [`ImageWorker`] 的名额分开：读可能要等慢插件，不能占着解码的名额，挡住别的图片处理（比如插件商店
/// 的图标）。
const MAX_THUMBNAIL_READS: usize = 4;
/// 读完的原图最多这么多张在内存里等解码或者正在解码。读的名额读完就还：解码慢不是来源的问题。满了不等，读完也白读，但内存有上限。
const MAX_THUMBNAIL_DECODES: usize = 8;
/// 等解码加解码最多这么久，比 [`ImageWorker`] 自己等子进程的 10 秒长：解码的毛病由它报。
const THUMBNAIL_DECODE_TIMEOUT: Duration = Duration::from_secs(30);
/// 一个工具结果的预览一共等多久（ADR-0020 的“预览”）。
const PREVIEW_BUDGET: Duration = Duration::from_secs(10);

/// 文件操作失败：类别加给人看的说明。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{}：{message}", kind.as_str())]
pub struct Failure {
    pub kind: FileFailure,
    pub message: String,
}

impl Failure {
    pub fn new(kind: FileFailure, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    fn unknown() -> Self {
        Self::new(FileFailure::FileUnknown, "没有这个文件")
    }

    /// 核心自己的问题（读写不了记录、起不了进程）。
    fn internal(message: impl Into<String>) -> Self {
        Self::new(FileFailure::Internal, message)
    }
}

impl From<ShareError> for Failure {
    fn from(e: ShareError) -> Self {
        let kind = match &e {
            ShareError::Gone => FileFailure::FileGone,
            ShareError::Refused(_) => FileFailure::PermissionDenied,
            ShareError::NoSpace(_) => FileFailure::StorageFull,
            ShareError::Io(_) => FileFailure::Internal,
        };
        Self::new(kind, e.to_string())
    }
}

impl From<RegistryError> for Failure {
    fn from(e: RegistryError) -> Self {
        tracing::error!(error = %e, "写编号记录失败");
        let full = matches!(
            &e,
            RegistryError::Store(StoreError::Io { source, .. })
                if [rustix::io::Errno::NOSPC, rustix::io::Errno::DQUOT]
                    .map(|full| Some(full.raw_os_error()))
                    .contains(&source.raw_os_error())
        );
        if full {
            return Self::new(FileFailure::StorageFull, e.to_string());
        }
        Self::internal(e.to_string())
    }
}

/// 启动时准备不好文件服务。
#[derive(Debug, thiserror::Error)]
pub enum OpenError {
    #[error(transparent)]
    Registry(#[from] RegistryError),
    #[error(transparent)]
    Dir(#[from] DirError),
    #[error("建不了删除文件用的回收槽：{0}")]
    Removal(#[source] ShareError),
    #[error("建立 {path} 失败：{source}")]
    Create {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// 文件服务的目录。
#[derive(Debug, Clone)]
pub struct FilesConfig {
    /// 核心数据目录下的 `core/files`。
    pub dir: PathBuf,
    /// 共享文件夹的根目录。
    pub shares_dir: PathBuf,
    /// 共享文件夹的属主（`PUID:PGID`）。
    pub owner: Owner,
}

/// 登记文件的 Bot。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BotRef {
    /// Bot 的名字（Matrix 用户名）。
    pub name: String,
    pub plugin: PluginId,
}

/// 一次下载的字节从哪来。
pub enum Content {
    File(Opened),
    /// 插件的内网地址的响应，只检查过状态码。
    Remote(hyper::Response<Body>),
    /// `resources/read` 读到的内容。
    Bytes(Vec<u8>),
    /// `HEAD` 不为问一下大小把整个资源读进内存：大小是插件声明的。
    Unread {
        size: u64,
    },
}

/// 审计的结果：文件删了，目录没能确认落盘，断电后可能回来。
pub const DELETED_UNSYNCED: &str = "ok_unsynced";

/// 文本预览的结果（[`Files::text_preview`]）：开头的内容和截断信息。
pub struct TextHead {
    pub content: String,
    /// 文件总大小；拿不到时为空，结果里如实说未知。
    pub total: Option<u64>,
    /// 实际读到的字节数。
    pub read: usize,
    pub truncated: bool,
}

/// [`Files::read_head`] 的中间结果；名额跟着读走，读完才还。
struct HeadRead {
    bytes: Vec<u8>,
    total: Option<u64>,
    truncated: bool,
    slot: ThumbnailSlot,
}

/// 审计日志的一行（`core/audit.jsonl`）。
#[derive(Debug, Serialize)]
pub struct FileEvent<'a> {
    pub event: &'static str,
    pub device: String,
    pub file: &'a str,
    /// 下载的字节范围。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub range: Option<String>,
    /// 授权时加进范围的 Bot。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bots: Option<&'a [String]>,
    /// `ok`、[`DELETED_UNSYNCED`] 或者失败的类别。
    pub outcome: &'a str,
}

pub struct Files {
    registry: Registry,
    shares: Shares,
    /// Bot 存文件的暂存目录（ADR-0027）。
    saving_dir: PathBuf,
    owner: Owner,
    uploads: uploads::Uploads,
    host: Arc<dyn FileHost>,
    http: crate::http::HttpClient,
    audit: Arc<Audit>,
    images: Arc<ImageWorker>,
    local_thumbnail_reads: Arc<tokio::sync::Semaphore>,
    plugin_thumbnail_reads: PluginSlots,
    thumbnail_decodes: Arc<tokio::sync::Semaphore>,
    thumbnail_read_timeout: Duration,
    thumbnail_slot_wait: Duration,
    thumbnail_decode_timeout: Duration,
    /// 一个工具结果的预览一共等多久。
    pub(crate) preview_budget: Duration,
    /// 测试用：拿着写锁时，读本地文件做缩略图的线程卡住，就像存储不回答。
    #[cfg(test)]
    local_reads_stalled: Arc<tokio::sync::RwLock<()>>,
}

impl Files {
    /// 读出编号记录，建好共享文件夹 `transfers`。
    pub fn open(
        config: &FilesConfig,
        host: Arc<dyn FileHost>,
        http: crate::http::HttpClient,
        audit: Arc<Audit>,
        images: Arc<ImageWorker>,
    ) -> Result<Self, OpenError> {
        for path in [&config.dir, &config.shares_dir] {
            fs::create_dir_all(path).map_err(|source| OpenError::Create {
                path: path.clone(),
                source,
            })?;
        }
        hostdir::ensure_owned_dir(&config.shares_dir.join(TRANSFERS), config.owner)?;
        let registry = Registry::open(config.dir.join("records"))?;
        let saving_dir = config.dir.join("saving");
        share_fs::saving::reset_staging(&saving_dir).map_err(|source| OpenError::Create {
            path: saving_dir.clone(),
            source,
        })?;
        let shares = Shares::new(config.shares_dir.clone());
        shares.prepare_removal().map_err(OpenError::Removal)?;
        // 找不回来不妨碍别的文件，下次启动再试。
        if let Err(e) = shares.recover_deleting() {
            tracing::error!(error = %e, "找回删除到一半的文件失败");
        }
        Ok(Self {
            registry,
            shares,
            saving_dir,
            owner: config.owner,
            uploads: uploads::Uploads::open(&config.dir, config.owner)?,
            host,
            http,
            audit,
            images,
            local_thumbnail_reads: Arc::new(tokio::sync::Semaphore::new(MAX_THUMBNAIL_READS)),
            plugin_thumbnail_reads: PluginSlots::new(),
            thumbnail_decodes: Arc::new(tokio::sync::Semaphore::new(MAX_THUMBNAIL_DECODES)),
            thumbnail_read_timeout: THUMBNAIL_READ_TIMEOUT,
            thumbnail_slot_wait: slots::SLOT_WAIT,
            thumbnail_decode_timeout: THUMBNAIL_DECODE_TIMEOUT,
            preview_budget: PREVIEW_BUDGET,
            #[cfg(test)]
            local_reads_stalled: Arc::default(),
        })
    }

    /// 同步的文件操作放到阻塞线程上：`fsync` 可能很慢，不能占着 tokio 的工作线程。
    async fn blocking<T: Send + 'static>(
        self: &Arc<Self>,
        work: impl FnOnce(&Self) -> Result<T, Failure> + Send + 'static,
    ) -> Result<T, Failure> {
        let files = self.clone();
        tokio::task::spawn_blocking(move || work(&files))
            .await
            .map_err(|e| Failure::internal(format!("文件操作的任务中止：{e}")))?
    }

    pub fn audit(&self, event: &FileEvent<'_>) {
        self.audit.record(event);
    }

    /// 查编号并核对访问范围，见模块说明里的表。
    pub fn authorize(&self, id: &FileId, who: &Accessor) -> Result<FileRecord, Failure> {
        let bot = matches!(who, Accessor::Bot(_));
        match self.registry.get(id, now_secs()) {
            Lookup::Found(record) if record.access.contains(who) => Ok(record),
            Lookup::Found(_) if bot => Err(Failure::unknown()),
            Lookup::Found(_) => Err(Failure::new(FileFailure::NotShared, "这个文件没有分享给你")),
            Lookup::Expired(record) if bot && record.access.contains(who) => Err(Failure::new(
                FileFailure::FileExpired,
                "这个编号没分享就过期了，再找一次文件",
            )),
            Lookup::Expired(_) | Lookup::Unknown => Err(Failure::unknown()),
        }
    }

    /// 文件信息。大小看当前的文件：主人改了文档，卡片显示当前的大小。
    pub async fn info(&self, id: &FileId, who: &Accessor) -> Result<FileInfo, Failure> {
        let record = self.authorize(id, who)?;
        let size = match &record.origin {
            Origin::Share { reference } | Origin::Upload { reference, .. } => {
                Some(self.open_share(reference.clone()).await?.snapshot.size)
            }
            Origin::Plugin { plugin, .. } => {
                self.host.check_plugin(plugin)?;
                record.declared_size
            }
        };
        let declared = record.mime_type.as_deref();
        let has_thumbnail = can_thumbnail(&record.name, declared)
            && size.is_none_or(|size| size <= thumbnail_input_limit(&record.origin));
        let mime_type = served_type(&record.name, declared)
            .map(str::to_owned)
            .or_else(|| declared.and_then(declared_type).map(str::to_owned));
        Ok(FileInfo {
            id: record.id,
            name: record.name,
            mime_type,
            size,
            source: record.source,
            has_thumbnail,
        })
    }

    async fn open_share(&self, reference: ShareRef) -> Result<Opened, Failure> {
        let shares = self.shares.clone();
        tokio::task::spawn_blocking(move || shares.open(&reference))
            .await
            .map_err(|e| Failure::internal(format!("打开文件的任务中止：{e}")))?
            .map_err(Failure::from)
    }

    /// 取字节。`request` 是要转给插件内网地址的方法和 `Range`。
    pub async fn content(
        &self,
        record: &FileRecord,
        request: RemoteRequest,
    ) -> Result<Content, Failure> {
        match &record.origin {
            Origin::Share { reference } | Origin::Upload { reference, .. } => {
                self.open_share(reference.clone()).await.map(Content::File)
            }
            Origin::Plugin {
                plugin,
                uri,
                internal_url,
            } => {
                self.host.check_plugin(plugin)?;
                match internal_url {
                    Some(url) => {
                        let url = InternalUrl::parse(url)
                            .map_err(|e| Failure::new(FileFailure::PermissionDenied, e))?;
                        let upstream = self.host.service_upstream(plugin, &url).await?;
                        resource::fetch(&self.http, &upstream, &url, &request)
                            .await
                            .map(Content::Remote)
                    }
                    None => {
                        self.host.check_mcp(plugin)?;
                        match record.declared_size {
                            Some(size) if size > MAX_RESOURCE_BYTES as u64 => Err(Failure::new(
                                FileFailure::ResourceTooLarge,
                                format!(
                                    "插件声明的大小超过 resources/read 的上限（{} MiB）",
                                    MAX_RESOURCE_BYTES >> 20
                                ),
                            )),
                            Some(size) if request.method == Method::HEAD => {
                                Ok(Content::Unread { size })
                            }
                            _ => self
                                .host
                                .read_resource(plugin, uri)
                                .await
                                .map(Content::Bytes),
                        }
                    }
                }
            }
        }
    }

    /// 缩略图：长边 800 像素的 JPEG，不带原文件的元数据，在单独的进程里解码。
    pub async fn thumbnail(&self, record: &FileRecord) -> Result<Vec<u8>, Failure> {
        self.scaled(record, Scale::Thumbnail).await
    }

    /// 文本文件的开头（`wonton__view_file` 的文本分支）：最多读 [`MAX_TEXT_PREVIEW_BYTES`] 字节，按内容判断是不是文本——
    /// 能按 UTF-8 解、没有 NUL 字节才算，不只看 MIME；截断边界上剩下的半个多字节字符不算数。PDF 等其他二进制返回
    /// `not_viewable`，这类读不了内容。访问范围和 file_changed 由调用方的 `for_bot` 把过；读取名额和期限沿用缩略图那套。
    pub async fn text_preview(&self, record: &FileRecord) -> Result<TextHead, Failure> {
        let not_text = |why: String| Failure::new(FileFailure::NotViewable, why);
        let reading = self.thumbnail_slot(&record.origin).await?;
        let read =
            tokio::time::timeout(self.thumbnail_read_timeout, self.read_head(record, reading))
                .await
                .map_err(|_| {
                    thumbnail_read_timed_out(&record.origin, self.thumbnail_read_timeout)
                })??;
        // 名额跟着读走，读完了就还；文本不用解码进程。
        drop(read.slot);
        if read.bytes.contains(&0) {
            return Err(not_text(format!(
                "{} 不是文本文件（内容里有 NUL 字节），读不了内容：PDF 等二进制文件这类只能下载或分享。",
                record.name
            )));
        }
        // 先按整段判断是不是文本：只有截断边界上的半个字符不算数，中间解不开就是二进制。
        if let Err(error) = std::str::from_utf8(&read.bytes) {
            let tail = read.bytes.len() - error.valid_up_to();
            if !(read.truncated && tail < 4) {
                return Err(not_text(format!(
                    "{} 不是文本文件：内容解不成 UTF-8，读不了内容。PDF 等二进制文件这类只能下载或分享。",
                    record.name
                )));
            }
        }
        // 内容最多上限字节；跨在边界上的那个多字节字符不完整，按替换字符算。
        let shown = &read.bytes[..read.bytes.len().min(MAX_TEXT_PREVIEW_BYTES)];
        let content = String::from_utf8_lossy(shown).into_owned();
        Ok(TextHead {
            content,
            total: read.total,
            read: shown.len(),
            truncated: read.truncated,
        })
    }

    /// 开头 [`MAX_TEXT_PREVIEW_BYTES`] 字节和截断信息。读 `上限+1` 个字节：读满就是截断，内容截到上限。
    /// 本地文件只读这么多；插件的流攒够就停，不往下拉。
    async fn read_head(
        &self,
        record: &FileRecord,
        reading: ThumbnailSlot,
    ) -> Result<HeadRead, Failure> {
        if let Origin::Share { reference } | Origin::Upload { reference, .. } = &record.origin {
            let shares = self.shares.clone();
            let reference = reference.clone();
            let bytes = tokio::task::spawn_blocking(move || {
                let opened = shares.open(&reference)?;
                let total = opened.snapshot.size;
                let mut file = opened.file.take(MAX_TEXT_PREVIEW_BYTES as u64 + 1);
                let mut bytes = Vec::new();
                file.read_to_end(&mut bytes)
                    .map_err(|e| Failure::internal(format!("读文件失败：{e}")))?;
                Ok::<_, Failure>((bytes, total))
            })
            .await
            .map_err(|e| Failure::internal(format!("读文件的任务中止：{e}")))??;
            let (bytes, total) = bytes;
            return Ok(HeadRead {
                truncated: bytes.len() > MAX_TEXT_PREVIEW_BYTES,
                bytes,
                total: Some(total),
                slot: reading,
            });
        }
        let content = self.content(record, RemoteRequest::whole()).await?;
        match content {
            Content::Remote(response) => {
                // 有 Content-Length 或声明大小时如实报总大小；都没有就如实说未知。
                let total = response
                    .headers()
                    .get(hyper::header::CONTENT_LENGTH)
                    .and_then(|value| value.to_str().ok())
                    .and_then(|value| value.parse().ok())
                    .or(record.declared_size);
                let bytes = resource::read_upto(response, MAX_TEXT_PREVIEW_BYTES + 1).await?;
                Ok(HeadRead {
                    truncated: bytes.len() > MAX_TEXT_PREVIEW_BYTES,
                    bytes,
                    total,
                    slot: reading,
                })
            }
            Content::Bytes(bytes) => {
                // 这条路（MCP resources/read）本来就整读进来了：截断标记按全长算，这里只留下头一段。
                let truncated = bytes.len() > MAX_TEXT_PREVIEW_BYTES;
                let mut bytes = bytes;
                bytes.truncate(MAX_TEXT_PREVIEW_BYTES + 1);
                Ok(HeadRead {
                    truncated,
                    bytes,
                    total: record.declared_size,
                    slot: reading,
                })
            }
            Content::File(_) | Content::Unread { .. } => {
                Err(Failure::internal("插件的资源读出来不该是文件句柄或未读"))
            }
        }
    }

    /// 缩小的图片，名额和期限在 [`Self::thumbnail`] 和 [`Self::preview`] 之间共用。
    async fn scaled(&self, record: &FileRecord, scale: Scale) -> Result<Vec<u8>, Failure> {
        let not_viewable = |why: String| Failure::new(FileFailure::NotViewable, why);
        if !can_thumbnail(&record.name, record.mime_type.as_deref()) {
            return Err(not_viewable("这个类型没有预览或缩略图".to_owned()));
        }
        let reading = self.thumbnail_slot(&record.origin).await?;
        let (data, reading) = tokio::time::timeout(
            self.thumbnail_read_timeout,
            self.read_for_thumbnail(record, reading),
        )
        .await
        .map_err(|_| thumbnail_read_timed_out(&record.origin, self.thumbnail_read_timeout))??;
        drop(reading);
        let decode_busy = || Failure::new(FileFailure::ServiceUnavailable, "图片解码忙");
        let _decoding = self
            .thumbnail_decodes
            .clone()
            .try_acquire_owned()
            .map_err(|_| decode_busy())?;
        let decoded = async {
            match scale {
                Scale::Thumbnail => self.images.thumbnail(&data).await,
                Scale::Preview => self.images.preview(&data).await,
            }
        };
        tokio::time::timeout(self.thumbnail_decode_timeout, decoded)
            .await
            .map_err(|_| decode_busy())?
            .map_err(|e| match e {
                ImageError::Refused(why) => not_viewable(format!("做不了预览或缩略图：{why}")),
                ImageError::Unavailable(why) => Failure::internal(why),
            })
    }

    /// 读取名额。读卡住时后来的请求不能一直等。本地的等不到不归给来源，这一张可能什么问题都没有；插件的见 [`PluginSlots::acquire`]。
    async fn thumbnail_slot(&self, origin: &Origin) -> Result<ThumbnailSlot, Failure> {
        let wait = self.thumbnail_slot_wait;
        match origin {
            Origin::Share { .. } | Origin::Upload { .. } => {
                let pool = self.local_thumbnail_reads.clone();
                tokio::time::timeout(wait, pool.acquire_owned())
                    .await
                    .map(|permit| ThumbnailSlot::Local(permit.expect("信号量不会关闭")))
                    .map_err(|_| Failure::new(FileFailure::ServiceUnavailable, "图片读取忙"))
            }
            Origin::Plugin { plugin, .. } => self
                .plugin_thumbnail_reads
                .acquire(plugin, wait)
                .await
                .map(ThumbnailSlot::Plugin),
        }
    }

    /// 原文件的字节和它占着的名额。
    async fn read_for_thumbnail(
        &self,
        record: &FileRecord,
        reading: ThumbnailSlot,
    ) -> Result<(Vec<u8>, ThumbnailSlot), Failure> {
        if let Origin::Share { reference } | Origin::Upload { reference, .. } = &record.origin {
            return self
                .read_local_for_thumbnail(reference.clone(), reading)
                .await;
        }
        let content = self
            .content(record, RemoteRequest::whole())
            .await
            .map_err(|e| match e.kind {
                FileFailure::ResourceTooLarge => too_big_for_thumbnail(),
                _ => e,
            })?;
        let bytes = match content {
            Content::Remote(response) => resource::read_limited(response, MAX_THUMBNAIL_INPUT)
                .await?
                .ok_or_else(too_big_for_thumbnail)?,
            Content::Bytes(bytes) if bytes.len() > MAX_THUMBNAIL_INPUT => {
                return Err(too_big_for_thumbnail());
            }
            Content::Bytes(bytes) => bytes,
            Content::File(_) | Content::Unread { .. } => {
                return Err(Failure::internal("插件的资源读出来不该是这样"));
            }
        };
        Ok((bytes, reading))
    }

    /// 阻塞的读放不下来：调用方超时之后它还在跑（存储卡住时可能一直不回来），所以名额跟着它走，真读完才还。打开也在里面，一样会卡。
    async fn read_local_for_thumbnail(
        &self,
        reference: ShareRef,
        reading: ThumbnailSlot,
    ) -> Result<(Vec<u8>, ThumbnailSlot), Failure> {
        let shares = self.shares.clone();
        #[cfg(test)]
        let stalled = self.local_reads_stalled.clone();
        tokio::task::spawn_blocking(move || {
            #[cfg(test)]
            let _stalled = stalled.blocking_read();
            let opened = shares.open(&reference)?;
            if opened.snapshot.size > MAX_THUMBNAIL_INPUT as u64 {
                return Err(too_big_for_thumbnail());
            }
            // 文件可能在查过大小之后变大。
            let mut file = opened.file.take(MAX_THUMBNAIL_INPUT as u64 + 1);
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes)
                .map_err(|e| Failure::internal(format!("读文件失败：{e}")))?;
            if bytes.len() > MAX_THUMBNAIL_INPUT {
                return Err(too_big_for_thumbnail());
            }
            Ok((bytes, reading))
        })
        .await
        .map_err(|e| Failure::internal(format!("读文件的任务中止：{e}")))?
    }

    /// 把这个房间的 Bot 加进访问范围，返回加了哪些。房间里没有 Bot 时什么都不做：App 发链接时不用先判断。
    pub async fn grant_room(
        self: &Arc<Self>,
        id: &FileId,
        room: &str,
    ) -> Result<Vec<String>, Failure> {
        let (id, room) = (id.clone(), room.to_owned());
        self.blocking(move |files| {
            files.authorize(&id, &Accessor::Owner)?;
            let bots = files.host.bots_in_room(&room)?;
            for bot in &bots {
                files
                    .registry
                    .grant_by_owner(&id, bot, now_secs())?
                    .ok_or_else(Failure::unknown)?;
            }
            Ok(bots)
        })
        .await
    }

    /// 单独收回一个 Bot 的访问：来源插件的授权收回时，主人亲手授权过的 Bot 不受影响，要靠这个。这个 Bot 再经工具找到同一个文件时，
    /// 按来源插件的授权算。
    pub async fn revoke_bot(self: &Arc<Self>, id: &FileId, bot: &str) -> Result<(), Failure> {
        let (id, bot) = (id.clone(), bot.to_owned());
        self.blocking(move |files| {
            files.authorize(&id, &Accessor::Owner)?;
            files
                .registry
                .revoke_bot(&id, &bot)?
                .ok_or_else(Failure::unknown)?;
            Ok(())
        })
        .await
    }

    /// 把链接发到“Wonton”私聊：主人所有设备都看得到卡片。
    pub async fn send_to_devices(&self, id: &FileId) -> Result<(), Failure> {
        self.authorize(id, &Accessor::Owner)?;
        self.host.notify_owner(&id.link()).await
    }

    /// 删掉编号。设备上传的文件核对后连文件一起删；已经换成别的文件时编号照删、文件不动，返回 `file_changed`。其他来源的
    /// 原文件不动。删除和审计在单独的任务里一起做：请求断了也要做完，文件删了就要有审计。
    pub async fn delete(self: &Arc<Self>, id: &FileId, device: String) -> Result<(), Failure> {
        let files = self.clone();
        let id = id.clone();
        tokio::spawn(async move {
            // 和放文件排同一个队：不然放到一半的文件可能删完之后才冒出来。
            let _turn = files.uploads.turn(&id).await;
            let task = id.clone();
            let removed = files.blocking(move |files| files.delete_now(&task)).await;
            let outcome = match &removed {
                Ok(outcome) => outcome,
                Err(e) => e.kind.as_str(),
            };
            files.audit(&FileEvent {
                event: "file_delete",
                device,
                file: id.as_str(),
                range: None,
                bots: None,
                outcome,
            });
            removed.map(|_| ())
        })
        .await
        .map_err(|e| Failure::internal(format!("删除文件的任务中止：{e}")))?
    }

    /// 返回审计要写的结果。目录没能确认落盘时记录也照删：文件这时已经没了，留着记录主人就删不掉这个编号，断电后文件回来的话审计里有据可查。
    fn delete_now(&self, id: &FileId) -> Result<&'static str, Failure> {
        let record = self.authorize(id, &Accessor::Owner)?;
        // 删失败了记录留着：文件还在，主人可以再删一次。
        let removed = match &record.origin {
            Origin::Upload { reference, placed } => {
                // 放好之后清理没做完（崩溃了）的残留先清掉：待完成记录和数据还在的话，下次清理会把删掉的文件再放回来。
                if !self.forget_now(id, true) {
                    return Err(Failure::internal("清不掉这个上传的残留，文件没有删"));
                }
                self.shares.delete_if_unchanged(reference, placed)?
            }
            _ => Deleted::AlreadyGone,
        };
        self.registry.remove(id)?;
        match removed {
            Deleted::Removed { durable: false } => Ok(DELETED_UNSYNCED),
            Deleted::Removed { durable: true } | Deleted::AlreadyGone => Ok("ok"),
            Deleted::Changed => Err(Failure::new(
                FileFailure::FileChanged,
                "这个路径已经是别的文件了：编号删了，文件没动",
            )),
        }
    }
}

/// 缩到多大。
#[derive(Clone, Copy)]
enum Scale {
    Thumbnail,
    Preview,
}

/// 占着的读取名额，读完才放下。
enum ThumbnailSlot {
    Local(#[allow(dead_code)] OwnedSemaphorePermit),
    Plugin(#[allow(dead_code)] PluginSlot),
}

/// 新的上传编号。网关在转发创建请求之前就定好，tusd 回 500、可能不回 `Location` 时也知道是哪个上传。
pub fn new_upload_id() -> FileId {
    new_id()
}

/// 128 位随机数：编号不可猜全靠它。
fn new_id() -> FileId {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).expect("操作系统随机数源不可用");
    FileId::from_bytes(bytes)
}

fn too_big_for_thumbnail() -> Failure {
    Failure::new(
        FileFailure::NotViewable,
        format!("文件超过 {} MiB，不做缩略图", MAX_THUMBNAIL_INPUT >> 20),
    )
}

/// 这个来源的文件多大还做得了缩略图：没有内网地址的插件资源只能经 `resources/read` 读。
fn thumbnail_input_limit(origin: &Origin) -> u64 {
    let limit = match origin {
        Origin::Plugin {
            internal_url: None, ..
        } => MAX_THUMBNAIL_INPUT.min(MAX_RESOURCE_BYTES),
        _ => MAX_THUMBNAIL_INPUT,
    };
    limit as u64
}

/// 插件读不完和 `/content` 一样算插件不可用；本地文件读不完不是插件的事，是核心这边（比如磁盘）出了问题。
fn thumbnail_read_timed_out(origin: &Origin, after: Duration) -> Failure {
    let why = format!("原文件 {after:?} 内没有读完");
    match origin {
        Origin::Plugin { .. } => Failure::new(FileFailure::PluginUnavailable, why),
        Origin::Share { .. } | Origin::Upload { .. } => Failure::internal(why),
    }
}

/// 审计里的设备：完整的 endpoint ID。
pub fn device_label(device: &EndpointId) -> String {
    device.to_string()
}

#[cfg(test)]
pub(crate) mod test_support;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod thumbnail_tests;
