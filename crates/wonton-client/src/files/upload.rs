//! 上传（tus 1.0，ADR-0020）。创建后先把编号写盘，再一次 PATCH 传完剩下的；断了问核心传到哪了接着传。最后一个 PATCH 回 204 才算文件
//! 可用，回答丢了就靠 `HEAD`：传完的 `Upload-Offset` 等于总长。

use std::fs::File;
use std::io::{self, SeekFrom};
use std::os::fd::FromRawFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, UNIX_EPOCH};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use futures_util::stream;
use reqwest::{Body, Method};
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use wonton_l10n::l10n;
use wonton_protocol::{CoreError, FileFailure, FileId, HomeStatus, UploadSource};

use super::FilesLink;
use super::api::{
    Api, Problem, TUS_RESUMABLE, TUS_VERSION, checked, header, interrupted, pausable,
};
use super::opener::FileOpenError;
use super::store::{Job, Record, SourceStamp};
use crate::l10n::{CATALOG, language};

const CHUNK: usize = 256 * 1024;
/// 这么久读不走一块就算卡住了：本地代理的缓冲满了说明隧道不动了。
const STALL: Duration = Duration::from_secs(60);
/// 字节发完后等多久：最后一个 PATCH 的回答要等核心把文件放进 `transfers`，大文件跨文件系统要复制。
const PLACEMENT: Duration = Duration::from_secs(10 * 60);
/// 发链接前等 Matrix 登录多久：App 重启后接着发时，恢复会话、第一次同步用不了一分钟。
/// 停在登录失败上的当场就走，不等。
const CHAT_SIGN_IN: Duration = Duration::from_secs(60);

pub(super) async fn run(link: &Arc<FilesLink>, record: Record) -> Result<(), Problem> {
    let Job::Upload {
        source,
        mime_type,
        room,
        stamp,
        uploaded,
    } = record.job
    else {
        unreachable!("record job is not an upload");
    };
    let api = Api::connect(&link.home)?;
    let id = record.id;
    // 传完了只差发链接时，原文件可能已经删了，也不用再读。记下传完的不再问核心：放进 `transfers` 后 tus 可能不认这个编号了。
    let finished = match &record.file {
        Some(_) if uploaded => true,
        Some(file) => Offset::query(&api, file).await?.complete(),
        None => false,
    };
    let file = if finished {
        record.file.expect("finished upload has a file id")
    } else {
        let (source, now) = open(link, &source).await?;
        if stamp.is_some_and(|stamp| stamp != now) {
            return Err(Problem::Failed(
                FileFailure::LocalSourceUnavailable,
                l10n!(CATALOG, language(), "files-upload-source-changed"),
            ));
        }
        let file = match record.file {
            Some(file) => file,
            None => create(&api, &record.name, mime_type.as_deref(), now).await?,
        };
        // 编号先写盘再传字节：App 这时被杀，下次还能接着传，不用从头来。
        link.store.update(&id, |record| {
            record.file = Some(file.clone());
            record.total_bytes = Some(now.size);
            if let Job::Upload { stamp, .. } = &mut record.job {
                *stamp = Some(now);
            }
        });
        send(link, &api, &id, &file, source).await?;
        file
    };
    let updated = link.store.update(&id, |record| {
        if let Job::Upload { uploaded, .. } = &mut record.job {
            *uploaded = true;
        }
    });
    if let Some(total) = updated.and_then(|record| record.total_bytes) {
        link.store.progress(&id, total);
    }
    if let Some(room) = room {
        deliver(link, &api, &file, &room).await?;
    }
    Ok(())
}

/// `HEAD` 的回答。
struct Offset {
    offset: u64,
    length: Option<u64>,
}

impl Offset {
    async fn query(api: &Api, file: &FileId) -> Result<Self, Problem> {
        let response = api
            .tus(Method::HEAD, &Api::upload_path(file))
            .send()
            .await
            .map_err(interrupted)?;
        let response = checked(response).await.map_err(pausable)?;
        let number = |name| header(response.headers(), name).and_then(|v| v.parse().ok());
        let offset = number("upload-offset").ok_or_else(|| {
            Problem::Failed(
                FileFailure::Protocol,
                l10n!(CATALOG, language(), "files-upload-head-no-offset"),
            )
        })?;
        Ok(Self {
            offset,
            length: number("upload-length"),
        })
    }

    fn complete(&self) -> bool {
        self.length == Some(self.offset)
    }
}

/// 打开原文件。`Opaque` 交给壳，在后台线程上调：壳可以阻塞。
async fn open(
    link: &Arc<FilesLink>,
    source: &UploadSource,
) -> Result<(File, SourceStamp), Problem> {
    let source = source.clone();
    let opener = link.opener();
    tokio::task::spawn_blocking(move || {
        let file = match source {
            UploadSource::Path { path } => File::open(&path).map_err(|e| source_error(&e))?,
            UploadSource::Opaque { handle } => {
                let opener = opener.ok_or_else(|| {
                    Problem::Failed(
                        FileFailure::LocalSourceUnavailable,
                        l10n!(CATALOG, language(), "files-upload-no-opener"),
                    )
                })?;
                let fd = opener.open_source(handle).map_err(opener_error)?;
                // 安全性：`FileOpener::open_source` 约定把描述符的所有权交给核心。
                unsafe { File::from_raw_fd(fd) }
            }
        };
        let meta = file.metadata().map_err(|e| source_error(&e))?;
        if !meta.is_file() {
            return Err(Problem::Failed(
                FileFailure::LocalSourceUnavailable,
                l10n!(CATALOG, language(), "files-upload-source-not-file"),
            ));
        }
        let modified_ns = meta
            .modified()
            .ok()
            .and_then(|at| at.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |since| {
                u64::try_from(since.as_nanos()).unwrap_or(u64::MAX)
            });
        let stamp = SourceStamp {
            size: meta.len(),
            modified_ns,
        };
        Ok((file, stamp))
    })
    .await
    .map_err(|e| {
        Problem::Failed(
            FileFailure::LocalIo,
            l10n!(
                CATALOG,
                language(),
                "files-upload-open-task-failed",
                reason = e.to_string()
            ),
        )
    })?
}

fn source_error(error: &io::Error) -> Problem {
    match error.kind() {
        io::ErrorKind::PermissionDenied => Problem::Failed(
            FileFailure::LocalPermissionDenied,
            l10n!(
                CATALOG,
                language(),
                "files-upload-source-denied",
                reason = error.to_string()
            ),
        ),
        io::ErrorKind::NotFound => Problem::Failed(
            FileFailure::LocalSourceUnavailable,
            l10n!(
                CATALOG,
                language(),
                "files-upload-source-missing",
                reason = error.to_string()
            ),
        ),
        _ => Problem::Failed(
            FileFailure::LocalIo,
            l10n!(
                CATALOG,
                language(),
                "files-upload-source-read",
                reason = error.to_string()
            ),
        ),
    }
}

fn opener_error(error: FileOpenError) -> Problem {
    let reason = match &error {
        FileOpenError::Unavailable { .. } => FileFailure::LocalSourceUnavailable,
        FileOpenError::PermissionDenied { .. } => FileFailure::LocalPermissionDenied,
        FileOpenError::Unexpected { .. } => FileFailure::LocalIo,
    };
    Problem::Failed(reason, error.to_string())
}

/// 创建上传。创建时不带字节：回答丢了的话，编号也跟着丢，核心会作废没人认领的上传。
async fn create(
    api: &Api,
    name: &str,
    mime_type: Option<&str>,
    stamp: SourceStamp,
) -> Result<FileId, Problem> {
    let mut metadata = format!("filename {}", STANDARD.encode(name));
    if let Some(mime_type) = mime_type {
        metadata.push_str(&format!(",filetype {}", STANDARD.encode(mime_type)));
    }
    let response = api
        .uploads()
        .header("Upload-Length", stamp.size)
        .header("Upload-Metadata", metadata)
        .send()
        .await
        .map_err(interrupted)?;
    let response = checked(response).await?;
    let file = header(response.headers(), "location")
        .and_then(|location| location.rsplit('/').next())
        .and_then(|id| id.parse().ok())
        .ok_or_else(|| {
            Problem::Failed(
                FileFailure::Protocol,
                l10n!(CATALOG, language(), "files-upload-create-no-location"),
            )
        })?;
    Ok(file)
}

/// 这次 PATCH 的动静：看门狗靠它判断卡没卡住。
struct Activity {
    last: Mutex<Instant>,
    finished: AtomicBool,
    /// 读原文件失败：这不是网络的问题，接着传也一样。
    failed: Mutex<Option<Problem>>,
}

impl Activity {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            last: Mutex::new(Instant::now()),
            finished: AtomicBool::new(false),
            failed: Mutex::new(None),
        })
    }

    fn touch(&self) {
        *self.last.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now();
    }

    async fn stalled(&self) {
        loop {
            tokio::time::sleep(Duration::from_secs(5)).await;
            let idle = self
                .last
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .elapsed();
            let limit = if self.finished.load(Ordering::Relaxed) {
                PLACEMENT
            } else {
                STALL
            };
            if idle > limit {
                return;
            }
        }
    }
}

/// 问传到哪了，传完剩下的。
async fn send(
    link: &Arc<FilesLink>,
    api: &Api,
    id: &str,
    file: &FileId,
    source: File,
) -> Result<(), Problem> {
    let at = Offset::query(api, file).await?;
    link.store.progress(id, at.offset);
    if at.complete() {
        return Ok(());
    }
    let total = at.length.ok_or_else(|| {
        Problem::Failed(
            FileFailure::Protocol,
            l10n!(CATALOG, language(), "files-upload-head-no-length"),
        )
    })?;
    let mut source = tokio::fs::File::from_std(source);
    source
        .seek(SeekFrom::Start(at.offset))
        .await
        .map_err(|e| source_error(&e))?;
    let activity = Activity::new();
    let remaining = total - at.offset;
    let body = body(
        link.clone(),
        id.to_owned(),
        source,
        at.offset,
        remaining,
        activity.clone(),
    );
    let request = api
        .stream(Method::PATCH, &Api::upload_path(file))
        .header(TUS_RESUMABLE, TUS_VERSION)
        .header("Upload-Offset", at.offset)
        .header(
            reqwest::header::CONTENT_TYPE,
            "application/offset+octet-stream",
        )
        .header(reqwest::header::CONTENT_LENGTH, remaining)
        .body(body);
    let sent = match tokio::select! {
        sent = request.send() => sent,
        () = activity.stalled() => {
            return Err(Problem::Interrupted(l10n!(
                CATALOG,
                language(),
                "files-upload-stalled"
            )));
        }
    } {
        Ok(sent) => sent,
        // 核心不读请求体就回 503 时连接先关，客户端可能先看到写失败：问一句进度，问出服务的毛病就照实说，
        // 别把这次说成等连接恢复。
        Err(error) => return Err(what_while_down(api, file, interrupted(error)).await),
    };
    let failed = activity
        .failed
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take();
    if let Some(failed) = failed {
        return Err(failed);
    }
    let response = tokio::select! {
        checked = checked(sent) => checked.map_err(pausable)?,
        () = activity.stalled() => {
            return Err(Problem::Interrupted(l10n!(
                CATALOG,
                language(),
                "files-upload-no-response"
            )));
        }
    };
    drop(response);
    Ok(())
}

fn body(
    link: Arc<FilesLink>,
    id: String,
    source: tokio::fs::File,
    start: u64,
    remaining: u64,
    activity: Arc<Activity>,
) -> Body {
    let state = (source, remaining, start);
    Body::wrap_stream(stream::unfold(
        state,
        move |(mut source, remaining, sent)| {
            let (link, id, activity) = (link.clone(), id.clone(), activity.clone());
            async move {
                if remaining == 0 {
                    return None;
                }
                let mut chunk = vec![0; usize::try_from(remaining).unwrap_or(CHUNK).min(CHUNK)];
                match source.read(&mut chunk).await {
                    Ok(read) if read > 0 => {
                        chunk.truncate(read);
                        let sent = sent + read as u64;
                        let remaining = remaining - read as u64;
                        link.store.progress(&id, sent);
                        activity.touch();
                        // 带 Content-Length 时 hyper 写完最后一块就不再要 body，走不到上面的 `remaining == 0`。
                        if remaining == 0 {
                            activity.finished.store(true, Ordering::Relaxed);
                        }
                        Some((Ok(chunk), (source, remaining, sent)))
                    }
                    Ok(_) => {
                        let short = io::Error::new(io::ErrorKind::UnexpectedEof, "source shrank");
                        *activity.failed.lock().unwrap_or_else(|e| e.into_inner()) =
                            Some(Problem::Failed(
                                FileFailure::LocalSourceUnavailable,
                                l10n!(CATALOG, language(), "files-upload-source-shrank"),
                            ));
                        Some((Err(short), (source, 0, sent)))
                    }
                    Err(e) => {
                        *activity.failed.lock().unwrap_or_else(|e| e.into_inner()) =
                            Some(source_error(&e));
                        Some((Err(e), (source, 0, sent)))
                    }
                }
            }
        },
    ))
}

/// 这次传不动时顺带问一句进度：问出服务的毛病（或者核心写明的失败）就照实说；答得上来、或者问也问不上，
/// 还是按原来的算。
async fn what_while_down(api: &Api, file: &FileId, problem: Problem) -> Problem {
    match Offset::query(api, file).await {
        Err(err @ Problem::ServicesDown(_)) | Err(err @ Problem::Failed(..)) => err,
        _ => problem,
    }
}

/// 传完后把这个房间的 Bot 加进访问范围，再把链接发进这个房间。App 被杀在这两步之间时，重启后再做一遍；授权本身可以重复，消息极少数
/// 情况下会多发一条。隧道断着时等连上再做；Matrix 还没登录好（App 重启后接着发链接时常见）等它一小会儿，别让主人点重发；
/// 其他的不自动再试，等主人点重发：聊天停在登录失败上、主人退出了房间这些情况，一直试也不会好。
async fn deliver(
    link: &Arc<FilesLink>,
    api: &Api,
    file: &FileId,
    room: &str,
) -> Result<(), Problem> {
    let sent = async {
        api.grant(file, room).await?;
        link.chat.signed_in_within(CHAT_SIGN_IN).await;
        link.chat
            .send_text_unopened(room, &file.link())
            .await
            .map_err(chat_problem)
    };
    sent.await.map_err(|problem| match problem {
        Problem::Interrupted(_) if link.home.view().status != HomeStatus::Connected => problem,
        other => {
            let (reason, message) = other.into_parts();
            Problem::LinkNotSent(reason, message)
        }
    })
}

/// 聊天那边的错算传输的哪一类。`NotReady`（还没登录 Matrix）、`Unauthorized`、`Upstream` 说的是手机这边的
/// 处境，借用了服务器那边的类别：两端的壳只显示说明，类别没有人用，就先不分（issue #19）。要分开得先加
/// `FileFailure`，两端的壳跟着改，等真有人要看类别再做。
fn chat_problem(error: CoreError) -> Problem {
    match error {
        CoreError::TransportUnavailable { reason } => Problem::Interrupted(reason),
        CoreError::NotReady { reason } => Problem::Failed(FileFailure::ServiceUnavailable, reason),
        CoreError::Unauthorized { reason } => Problem::Failed(FileFailure::Unauthenticated, reason),
        CoreError::InvalidInput { reason } => Problem::Failed(FileFailure::InvalidInput, reason),
        CoreError::Storage { reason } => Problem::Failed(FileFailure::LocalIo, reason),
        CoreError::Upstream { reason } => Problem::Failed(FileFailure::UpstreamError, reason),
        CoreError::ServerTooOld { reason, .. } => {
            Problem::Failed(FileFailure::UpstreamError, reason)
        }
    }
}

/// 取消：连核心那边传了一半的内容一起删。删不掉也不要紧，核心 24 小时后自己清。
pub(super) async fn abort(api: Api, file: FileId) {
    let response = api
        .tus(Method::DELETE, &Api::upload_path(&file))
        .send()
        .await;
    let done = match response {
        Ok(response) => checked(response).await.map(drop),
        Err(e) => Err(interrupted(e)),
    };
    if let Err(problem) = done {
        tracing::info!(file = %file, ?problem, "could not delete the partial upload on the home server");
    }
}
