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
mod operations;
mod private;
pub mod record;
pub mod resource;
mod saving;
pub mod share_fs;
mod slots;
mod thumbnails;
pub mod uploads;

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use hyper::Method;
use iroh::EndpointId;
use serde::Serialize;
use wonton_protocol::{FileFailure, FileId, FileInfo};

pub use host::{CoreHost, FileHost};

use crate::l10n;
use crate::mcp::audit::Audit;
use crate::plugin::PluginId;
use crate::plugin::hostdir::{DirError, Owner};
use crate::plugin::image_worker::{ImageError, ImageWorker};
use crate::plugin::languages::Languages;
use crate::plugin::manager::now_secs;
use crate::plugin::store::StoreError;
use content::{can_thumbnail, declared_type, served_type};
use record::{Accessor, FileRecord, Lookup, Origin, Registry, RegistryError};
use resource::{InternalUrl, RemoteRequest};
#[cfg(test)]
use share_fs::TRANSFERS;
use share_fs::{Opened, ShareError, ShareRef, Shares};
use slots::PluginSlots;
use thumbnails::{
    MAX_THUMBNAIL_DECODES, MAX_THUMBNAIL_READS, Scale, THUMBNAIL_DECODE_TIMEOUT,
    thumbnail_input_limit, thumbnail_read_timed_out, too_big_for_parsing,
};

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

    fn unknown(langs: &Languages) -> Self {
        Self::new(
            FileFailure::FileUnknown,
            l10n::text(langs, "files-not-found", &[]),
        )
    }

    /// 核心自己的问题（读写不了记录、起不了进程）。
    fn internal(message: impl Into<String>) -> Self {
        Self::new(FileFailure::Internal, message)
    }
}

impl From<ShareError> for Failure {
    fn from(e: ShareError) -> Self {
        Self::new(share_kind(&e), e.to_string())
    }
}

/// `ShareError` 的类别。
fn share_kind(e: &ShareError) -> FileFailure {
    match e {
        ShareError::Gone => FileFailure::FileGone,
        ShareError::Refused(_) => FileFailure::PermissionDenied,
        ShareError::NoSpace(_) => FileFailure::StorageFull,
        ShareError::Io(_) => FileFailure::Internal,
    }
}

/// 和 [`From<ShareError>`] 一样，但拿得到请求的语言时，“文件已被移动或删除”按语言说。
fn share_failure(e: ShareError, langs: &Languages) -> Failure {
    let message = match &e {
        ShareError::Gone => l10n::text(langs, "files-moved-or-deleted", &[]),
        _ => e.to_string(),
    };
    Failure::new(share_kind(&e), message)
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
    /// Data root containing core/home; independent from the file registry directory.
    pub data_dir: PathBuf,
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

/// 从 PDF 抽出来的文字（[`Files::pdf_text`]）。
pub struct PdfText {
    pub text: String,
    /// 文字里读到第几页（空白页也算读过）。
    pub pages_read: u32,
    pub pages_total: u32,
    pub truncated: bool,
}

/// 视频的时长和分辨率（[`Files::video_info`]）。
pub struct VideoMeta {
    pub width: u32,
    pub height: u32,
    pub duration_ms: u64,
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
    data_dir: PathBuf,
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
    /// Open the registry and recover interrupted deletions in existing private homes.
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
        let registry = Registry::open(config.dir.join("records"))?;
        let saving_dir = config.dir.join("saving");
        share_fs::saving::reset_staging(&saving_dir).map_err(|source| OpenError::Create {
            path: saving_dir.clone(),
            source,
        })?;
        let shares = Shares::new(config.shares_dir.clone());
        Self::recover_homes(&config.data_dir).map_err(OpenError::Removal)?;
        Ok(Self {
            registry,
            shares,
            data_dir: config.data_dir.clone(),
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
    pub fn authorize(
        &self,
        id: &FileId,
        who: &Accessor,
        langs: &Languages,
    ) -> Result<FileRecord, Failure> {
        self.host.check_person(who)?;
        let bot = matches!(who, Accessor::Bot(_));
        match self.registry.get(id, now_secs()) {
            Lookup::Found(record) if record.access.contains(who) => Ok(record),
            Lookup::Found(_) if bot => Err(Failure::unknown(langs)),
            Lookup::Found(_) => Err(Failure::new(
                FileFailure::NotShared,
                l10n::text(langs, "files-not-shared", &[]),
            )),
            Lookup::Expired(record) if bot && record.access.contains(who) => Err(Failure::new(
                FileFailure::FileExpired,
                l10n::text(langs, "files-share-expired", &[]),
            )),
            Lookup::Expired(_) | Lookup::Unknown => Err(Failure::unknown(langs)),
        }
    }

    /// 文件信息。大小看当前的文件：主人改了文档，卡片显示当前的大小。
    pub async fn info(
        &self,
        id: &FileId,
        who: &Accessor,
        langs: &Languages,
    ) -> Result<FileInfo, Failure> {
        let record = self.authorize(id, who, langs)?;
        let size = match &record.origin {
            Origin::Share { reference } | Origin::Upload { reference, .. } => Some(
                self.open_origin(&record.origin, reference.clone(), langs)
                    .await?
                    .snapshot
                    .size,
            ),
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

    async fn open_share(&self, reference: ShareRef, langs: &Languages) -> Result<Opened, Failure> {
        let shares = self.shares.clone();
        tokio::task::spawn_blocking(move || shares.open(&reference))
            .await
            .map_err(|e| Failure::internal(format!("打开文件的任务中止：{e}")))?
            .map_err(|e| share_failure(e, langs))
    }

    /// 取字节。`request` 是要转给插件内网地址的方法和 `Range`。
    pub async fn content(
        &self,
        record: &FileRecord,
        request: RemoteRequest,
        langs: &Languages,
    ) -> Result<Content, Failure> {
        match &record.origin {
            Origin::Share { reference } | Origin::Upload { reference, .. } => self
                .open_origin(&record.origin, reference.clone(), langs)
                .await
                .map(Content::File),
            Origin::Plugin {
                plugin,
                uri,
                internal_url,
            } => {
                self.host.check_plugin(plugin)?;
                match internal_url {
                    Some(url) => {
                        let url = InternalUrl::parse(url, langs)
                            .map_err(|e| Failure::new(FileFailure::PermissionDenied, e))?;
                        let upstream = self.host.service_upstream(plugin, &url).await?;
                        resource::fetch(&self.http, &upstream, &url, &request, langs)
                            .await
                            .map(Content::Remote)
                    }
                    None => {
                        self.host.check_mcp(plugin)?;
                        match record.declared_size {
                            Some(size) if size > MAX_RESOURCE_BYTES as u64 => Err(Failure::new(
                                FileFailure::ResourceTooLarge,
                                l10n::text(
                                    langs,
                                    "files-declared-size-over-limit",
                                    &[(
                                        "limit",
                                        wonton_l10n::Arg::from((MAX_RESOURCE_BYTES >> 20) as i64),
                                    )],
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
    pub async fn thumbnail(
        &self,
        record: &FileRecord,
        langs: &Languages,
    ) -> Result<Vec<u8>, Failure> {
        self.scaled(record, Scale::Thumbnail, langs).await
    }

    /// 文本文件的开头（`wonton__view_file` 的文本分支）：最多读 [`MAX_TEXT_PREVIEW_BYTES`] 字节，按内容判断是不是文本——
    /// 能按 UTF-8 解、没有 NUL 字节才算，不只看 MIME；截断边界上剩下的半个多字节字符不算数。PDF 等其他二进制返回
    /// `not_viewable`，这类读不了内容。访问范围和 file_changed 由调用方的 `for_bot` 把过；读取名额和期限沿用缩略图那套。
    pub async fn text_preview(
        &self,
        record: &FileRecord,
        langs: &Languages,
    ) -> Result<TextHead, Failure> {
        let not_text = |why: String| Failure::new(FileFailure::NotViewable, why);
        let reading = self.thumbnail_slot(&record.origin, langs).await?;
        let read = tokio::time::timeout(
            self.thumbnail_read_timeout,
            self.read_head(record, reading, langs),
        )
        .await
        .map_err(|_| {
            thumbnail_read_timed_out(&record.origin, self.thumbnail_read_timeout, langs)
        })??;
        // 名额跟着读走，读完了就还；文本不用解码进程。
        drop(read.slot);
        if read.bytes.contains(&0) {
            return Err(not_text(l10n::text(
                langs,
                "files-not-text-nul",
                &[("name", record.name.as_str().into())],
            )));
        }
        // 先按整段判断是不是文本：只有截断边界上的半个字符不算数，中间解不开就是二进制。
        if let Err(error) = std::str::from_utf8(&read.bytes) {
            let tail = read.bytes.len() - error.valid_up_to();
            if !(read.truncated && tail < 4) {
                return Err(not_text(l10n::text(
                    langs,
                    "files-not-text-utf8",
                    &[("name", record.name.as_str().into())],
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

    /// PDF 的文字层（`wonton__view_file` 的 PDF 分支，ADR-0104）：整份读进内存（上限同缩略图），在图片助手里按页抽，
    /// 解析崩了报 `not_viewable`。名额和期限沿用缩略图那套，忙和超时报 `service_unavailable`。
    pub async fn pdf_text(
        &self,
        record: &FileRecord,
        langs: &Languages,
    ) -> Result<PdfText, Failure> {
        let data = self.view_input(record, langs).await?;
        let _decoding = self.decode_slot(langs)?;
        let extracted =
            tokio::time::timeout(self.thumbnail_decode_timeout, self.images.pdf_text(&data))
                .await
                .map_err(|_| view_busy(langs))?
                .map_err(pdf_failure)?;
        Ok(PdfText {
            text: extracted.text,
            pages_read: extracted.pages_read,
            pages_total: extracted.pages_total,
            truncated: extracted.truncated,
        })
    }

    /// 视频的时长和分辨率（`wonton__view_file` 的视频分支，ADR-0104）：和头像的检查不同，不要求正方、10 分钟内或 H.264。
    /// ISO-BMFF 的读取核心里本来就有，只在阻塞线程上跑。
    pub async fn video_info(
        &self,
        record: &FileRecord,
        langs: &Languages,
    ) -> Result<VideoMeta, Failure> {
        let data = self.view_input(record, langs).await?;
        let _decoding = self.decode_slot(langs)?;
        let parse = async {
            let langs = langs.clone();
            tokio::task::spawn_blocking(move || {
                crate::bot_media::probe_video(&data).map_err(|_| {
                    Failure::new(
                        FileFailure::NotViewable,
                        l10n::text(&langs, "files-video-unreadable", &[]),
                    )
                })
            })
            .await
            .map_err(|e| Failure::internal(format!("解析视频的任务中止：{e}")))?
        };
        let info = tokio::time::timeout(self.thumbnail_decode_timeout, parse)
            .await
            .map_err(|_| view_busy(langs))??;
        Ok(VideoMeta {
            width: info.width,
            height: info.height,
            duration_ms: info.duration_ms,
        })
    }

    /// 要看内容的整份读取：名额和期限同缩略图，超过上限报 `resource_too_large`。
    async fn view_input(&self, record: &FileRecord, langs: &Languages) -> Result<Vec<u8>, Failure> {
        let reading = self.thumbnail_slot(&record.origin, langs).await?;
        let (bytes, reading) = tokio::time::timeout(
            self.thumbnail_read_timeout,
            self.read_whole(record, reading, langs, too_big_for_parsing),
        )
        .await
        .map_err(|_| {
            thumbnail_read_timed_out(&record.origin, self.thumbnail_read_timeout, langs)
        })??;
        drop(reading);
        Ok(bytes)
    }

    /// 解析类工作的名额：满了如实报忙，不排着队等。
    fn decode_slot(&self, langs: &Languages) -> Result<tokio::sync::OwnedSemaphorePermit, Failure> {
        self.thumbnail_decodes
            .clone()
            .try_acquire_owned()
            .map_err(|_| view_busy(langs))
    }
}

/// 抽 PDF、读视频的名额满了或者到点了。
fn view_busy(langs: &Languages) -> Failure {
    Failure::new(
        FileFailure::ServiceUnavailable,
        l10n::text(langs, "files-view-busy", &[]),
    )
}

/// 抽 PDF 时图片助手那边的错误：内容的问题重试也一样（`not_viewable`），超时和起不来按 ADR-0104 的类别报。
fn pdf_failure(error: ImageError) -> Failure {
    match error {
        ImageError::Refused(why) => Failure::new(FileFailure::NotViewable, why),
        ImageError::TimedOut(why) => Failure::new(FileFailure::ServiceUnavailable, why),
        ImageError::Unavailable(why) => Failure::internal(why),
    }
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

/// 审计里的设备：完整的 endpoint ID。
pub fn device_label(device: &EndpointId) -> String {
    device.to_string()
}

#[cfg(test)]
pub(crate) mod test_support;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_sources;
#[cfg(test)]
mod thumbnail_tests;
