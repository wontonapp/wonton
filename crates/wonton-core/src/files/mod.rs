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
use crate::plugin::hostdir::{self, DirError, Owner};
use crate::plugin::image_worker::ImageWorker;
use crate::plugin::languages::Languages;
use crate::plugin::manager::now_secs;
use crate::plugin::store::StoreError;
use content::{can_thumbnail, declared_type, served_type};
use record::{Accessor, FileRecord, Lookup, Origin, Registry, RegistryError};
use resource::{InternalUrl, RemoteRequest};
use share_fs::{Deleted, Opened, ShareError, ShareRef, Shares, TRANSFERS};
use slots::PluginSlots;
use thumbnails::{
    MAX_THUMBNAIL_DECODES, MAX_THUMBNAIL_READS, Scale, THUMBNAIL_DECODE_TIMEOUT,
    thumbnail_input_limit, thumbnail_read_timed_out,
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
    pub fn authorize(
        &self,
        id: &FileId,
        who: &Accessor,
        langs: &Languages,
    ) -> Result<FileRecord, Failure> {
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
                self.open_share(reference.clone(), langs)
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
                .open_share(reference.clone(), langs)
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

    /// 把这个房间的 Bot 加进访问范围，返回加了哪些。房间里没有 Bot 时也会清掉过期：App 授权之后马上发链接，这时链接就算发出去了。
    /// 和到期清理排同一个队：清理已经取了过期名单也不会在授权之后把文件删掉。
    pub async fn grant_room(
        self: &Arc<Self>,
        id: &FileId,
        room: &str,
        langs: &Languages,
    ) -> Result<Vec<String>, Failure> {
        let (task, room, langs) = (id.clone(), room.to_owned(), langs.clone());
        let turn = self.uploads.turn(id).await;
        let bots = self
            .blocking(move |files| {
                files.authorize(&task, &Accessor::Owner, &langs)?;
                // 主人已经在范围里，`grant` 也负责清掉过期时间（ADR-0020 的“生命周期”）。
                files
                    .registry
                    .grant(&task, Accessor::Owner, now_secs())?
                    .ok_or_else(|| Failure::unknown(&langs))?;
                let bots = files.host.bots_in_room(&room)?;
                for bot in &bots {
                    files
                        .registry
                        .grant_by_owner(&task, bot, now_secs())?
                        .ok_or_else(|| Failure::unknown(&langs))?;
                }
                Ok(bots)
            })
            .await;
        drop(turn);
        bots
    }

    /// 单独收回一个 Bot 的访问：来源插件的授权收回时，主人亲手授权过的 Bot 不受影响，要靠这个。这个 Bot 再经工具找到同一个文件时，
    /// 按来源插件的授权算。
    pub async fn revoke_bot(
        self: &Arc<Self>,
        id: &FileId,
        bot: &str,
        langs: &Languages,
    ) -> Result<(), Failure> {
        let (id, bot, langs) = (id.clone(), bot.to_owned(), langs.clone());
        self.blocking(move |files| {
            files.authorize(&id, &Accessor::Owner, &langs)?;
            files
                .registry
                .revoke_bot(&id, &bot)?
                .ok_or_else(|| Failure::unknown(&langs))?;
            Ok(())
        })
        .await
    }

    /// 把链接发到“Wonton”私聊：主人所有设备都看得到卡片。发之前先清掉过期（和到期清理排同一个队），发不出去时文件多留一阵也好过链接被删掉。
    pub async fn send_to_devices(
        self: &Arc<Self>,
        id: &FileId,
        langs: &Languages,
    ) -> Result<(), Failure> {
        let (task, langs) = (id.clone(), langs.clone());
        let turn = self.uploads.turn(id).await;
        let granted = self
            .blocking(move |files| {
                files.authorize(&task, &Accessor::Owner, &langs)?;
                files
                    .registry
                    .grant(&task, Accessor::Owner, now_secs())?
                    .ok_or_else(|| Failure::unknown(&langs))?;
                Ok(())
            })
            .await;
        drop(turn);
        granted?;
        self.host.notify_owner(&id.link()).await
    }

    /// 删掉编号。设备上传的文件核对后连文件一起删；已经换成别的文件时编号照删、文件不动，返回 `file_changed`。其他来源的
    /// 原文件不动。删除和审计在单独的任务里一起做：请求断了也要做完，文件删了就要有审计。
    pub async fn delete(
        self: &Arc<Self>,
        id: &FileId,
        device: String,
        langs: &Languages,
    ) -> Result<(), Failure> {
        let files = self.clone();
        let id = id.clone();
        let langs = langs.clone();
        tokio::spawn(async move {
            // 和放文件排同一个队：不然放到一半的文件可能删完之后才冒出来。
            let _turn = files.uploads.turn(&id).await;
            let task = id.clone();
            let langs = langs.clone();
            let removed = files
                .blocking(move |files| files.delete_now(&task, &langs))
                .await;
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
    fn delete_now(&self, id: &FileId, langs: &Languages) -> Result<&'static str, Failure> {
        let record = self.authorize(id, &Accessor::Owner, langs)?;
        // 删失败了记录留着：文件还在，主人可以再删一次。
        let removed = match &record.origin {
            Origin::Upload { reference, placed } => {
                // 放好之后清理没做完（崩溃了）的残留先清掉：待完成记录和数据还在的话，下次清理会把删掉的文件再放回来。
                if !self.forget_now(id, true) {
                    return Err(Failure::new(
                        FileFailure::Internal,
                        l10n::text(langs, "files-residual-undeletable", &[]),
                    ));
                }
                self.shares
                    .delete_if_unchanged(reference, placed, langs)
                    .map_err(|e| share_failure(e, langs))?
            }
            _ => Deleted::AlreadyGone,
        };
        self.registry.remove(id)?;
        match removed {
            Deleted::Removed { durable: false } => Ok(DELETED_UNSYNCED),
            Deleted::Removed { durable: true } | Deleted::AlreadyGone => Ok("ok"),
            Deleted::Changed => Err(Failure::new(
                FileFailure::FileChanged,
                l10n::text(langs, "files-path-collision", &[]),
            )),
        }
    }

    /// 传完但链接没发出去的上传到期了：和 App 里删同一条核对，记录跟着删。调用方排着这个上传的队，没有请求头，里面的错误按英文说。
    /// 开始之后才授权出去（链接又发了）的不删；名字上已经换成主人的文件时不再过期，留着记录。返回审计要写的结果。
    fn delete_expired_now(&self, id: &FileId, now: u64) -> Result<&'static str, Failure> {
        let record = match self.registry.get(id, now) {
            Lookup::Expired(record) => record,
            // 已经被删掉或者刚刚授权出去。
            _ => return Ok("kept"),
        };
        let Origin::Upload { reference, placed } = &record.origin else {
            return Ok("kept");
        };
        let langs = Languages::from_header(None);
        if !self.forget_now(id, true) {
            return Err(Failure::internal(l10n::text(
                &langs,
                "files-residual-undeletable",
                &[],
            )));
        }
        match self.shares.delete_if_unchanged(reference, placed, &langs) {
            Ok(Deleted::AlreadyGone) => {
                self.registry.remove(id)?;
                Ok("expired")
            }
            Ok(Deleted::Removed { durable }) => {
                // 文件已经没了，记录留着就删不掉这个编号。
                self.registry.remove(id)?;
                Ok(if durable { "expired" } else { DELETED_UNSYNCED })
            }
            Ok(Deleted::Changed) => {
                self.registry.keep(id)?;
                Ok("file_changed")
            }
            Err(e) => Err(share_failure(e, &langs)),
        }
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
