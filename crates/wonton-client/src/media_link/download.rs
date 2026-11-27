//! 离线下载的执行（ADR-0113 第 3 节）：把核心的下载任务落成这台设备上的文件。
//!
//! 字节经核心的 `GET /tasks/{id}/items/{item}/content` 取，先写 `<任务>.part`，收全了才改名，下载目录里出现的
//! 总是完整的文件。接着下时带 `Range: bytes=<已有>-` 和 `If-Range: <上次的 ETag>`：回 206 且起点对得上就接着写；
//! 回 200（源变了、不支持续传）就从 0 写；206 起点对不上或 416（半截比源还长）就丢掉半截、从 0 重来一次。
//! 没有 ETag 不续：`If-Range` 没得比，源变了就会拼出坏文件。
//!
//! 进度节流报给核心，写完报完成；没成的报失败，退避由核心按类别定（报不上就算了：`.part` 留着，下次和
//! 服务器对齐时接着下）。每个任务一个 `<任务>.json` 记条目、ETag、总长和写完的文件名，离线播放按条目 id
//! 从这里找文件。

use std::collections::HashMap;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use reqwest::header::{CONTENT_DISPOSITION, CONTENT_RANGE, CONTENT_TYPE, ETAG, IF_RANGE, RANGE};
use reqwest::{Method, Response, StatusCode};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use tokio::runtime::Handle;
use tokio::task::AbortHandle;
use wonton_protocol::media::api::{
    TaskProgressReport, task_complete_path, task_failure_path, task_item_content_path,
    task_progress_path,
};
use wonton_protocol::media::{
    MediaFailure, MediaFailureCode, MediaItemRef, MediaTask, TaskId, TaskKind, TaskState,
    TaskTarget,
};

use super::api::{MediaApi, checked, failure, unreachable};
use crate::l10n::text;

const DIR: &str = "media-downloads";
/// 写完的文件放在下载目录的这个子目录里：和记录（`<任务>.json`）、半截（`<任务>.part`）分开，
/// 提供方给什么扩展名都撞不上它们。
const FILES: &str = "files";
/// 进度最多这么久报一次（第 3 节：传输每 1 秒一次）。
const REPORT_EVERY: Duration = Duration::from_secs(1);
/// 这么久收不到一块就算断了。
const STALL: Duration = Duration::from_secs(60);

/// 一个下载任务在本机的记录。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Record {
    pub(crate) task: TaskId,
    pub(crate) item: MediaItemRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) etag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) total: Option<u64>,
    /// 写完后的文件相对下载目录的路径；写完前为空。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) file: Option<String>,
}

/// 一条条目在这台设备上的离线副本。
#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum OfflineState {
    /// 下了一部分；`total` 不知道时为空。
    Downloading { done: u64, total: Option<u64> },
    /// 写完了，`path` 是本机文件的绝对路径，播放器直接放。
    Ready { path: String },
}

/// 这一次取字节的结果：收全了，或者半截接不上、要从 0 再来。
enum Fetched {
    Done(PathBuf),
    Restart,
}

/// 这台设备上的下载：目录和在跑的任务。
pub(crate) struct Downloads {
    dir: PathBuf,
    running: Mutex<HashMap<TaskId, AbortHandle>>,
}

/// 该由这台设备接着下的状态。暂停、等网络、等电源由核心改回 `Queued` 后再下。
fn runnable(state: &TaskState) -> bool {
    matches!(
        state,
        TaskState::Queued | TaskState::Preparing | TaskState::Running
    )
}

impl Downloads {
    pub(crate) fn open(scope_dir: &Path) -> Arc<Self> {
        Arc::new(Self {
            dir: scope_dir.join(DIR),
            running: Mutex::new(HashMap::new()),
        })
    }

    fn part(&self, task: TaskId) -> PathBuf {
        self.dir.join(format!("{task}.part"))
    }

    fn record_path(&self, task: TaskId) -> PathBuf {
        self.dir.join(format!("{task}.json"))
    }

    fn read_record(&self, task: TaskId) -> Option<Record> {
        let bytes = std::fs::read(self.record_path(task)).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    /// 先写临时文件再改名：写到一半被杀，旧记录还在。
    fn write_record(&self, record: &Record) -> Result<(), MediaFailure> {
        std::fs::create_dir_all(&self.dir).map_err(|e| local(&e))?;
        let tmp = self.dir.join(format!("{}.json.tmp", record.task));
        let bytes = serde_json::to_vec(record).expect("download records always serialize");
        std::fs::write(&tmp, bytes).map_err(|e| local(&e))?;
        std::fs::rename(&tmp, self.record_path(record.task)).map_err(|e| local(&e))
    }

    /// 按服务器的任务表对齐：这台设备的下载里该下的起起来，不该下的停掉，取消了的删掉半截。
    pub(crate) fn reconcile(
        self: &Arc<Self>,
        runtime: &Handle,
        api: &MediaApi,
        device: &str,
        tasks: &[MediaTask],
    ) {
        for task in tasks {
            let TaskTarget::Download(item) = &task.target else {
                continue;
            };
            if task.kind != TaskKind::Download || task.device != device {
                continue;
            }
            if runnable(&task.state) {
                self.start(runtime, api.clone(), task.id, item.clone());
            } else {
                self.stop(task.id);
                if matches!(task.state, TaskState::Cancelled) {
                    self.discard(task.id);
                }
            }
        }
    }

    fn start(self: &Arc<Self>, runtime: &Handle, api: MediaApi, task: TaskId, item: MediaItemRef) {
        let mut running = self.running.lock().unwrap_or_else(|p| p.into_inner());
        if running.get(&task).is_some_and(|h| !h.is_finished()) {
            return;
        }
        let downloads = self.clone();
        let handle = runtime.spawn(async move {
            if let Err(failure) = downloads.run(&api, task, item).await {
                tracing::warn!(%task, code = ?failure.code, error = %failure.message, "download did not finish");
            }
        });
        running.insert(task, handle.abort_handle());
    }

    fn stop(&self, task: TaskId) {
        let handle = self
            .running
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&task);
        if let Some(handle) = handle {
            handle.abort();
        }
    }

    /// 取消的任务：删半截和记录。写完的文件不在这里删，删离线副本是单独的动作。
    fn discard(&self, task: TaskId) {
        if self.read_record(task).is_some_and(|r| r.file.is_some()) {
            return;
        }
        let _ = std::fs::remove_file(self.part(task));
        let _ = std::fs::remove_file(self.record_path(task));
    }

    /// 下完一个任务并报给核心。写完了的（上次改了名、完成没报上）直接再报一次完成。
    pub(crate) async fn run(
        &self,
        api: &MediaApi,
        task: TaskId,
        item: MediaItemRef,
    ) -> Result<PathBuf, MediaFailure> {
        let result = async {
            if let Fetched::Done(path) = self.fetch(api, task, &item, true).await? {
                return Ok(path);
            }
            match self.fetch(api, task, &item, false).await? {
                Fetched::Done(path) => Ok(path),
                Fetched::Restart => Err(failure(
                    MediaFailureCode::Protocol,
                    text("video-download-restart-mismatch", &[]),
                )),
            }
        }
        .await;
        match &result {
            Ok(_) => {
                api.post_empty(&task_complete_path(task), &serde_json::json!({}))
                    .await?;
            }
            Err(failure) => {
                if let Err(report) = api.post_empty(&task_failure_path(task), failure).await {
                    tracing::warn!(%task, error = %report.message, "download failed and the failure could not be reported to the core");
                }
            }
        }
        result
    }

    async fn fetch(
        &self,
        api: &MediaApi,
        task: TaskId,
        item: &MediaItemRef,
        may_resume: bool,
    ) -> Result<Fetched, MediaFailure> {
        let mut record = self.read_record(task).unwrap_or_else(|| Record {
            task,
            item: item.clone(),
            etag: None,
            total: None,
            file: None,
        });
        if let Some(file) = &record.file {
            let path = self.dir.join(file);
            if path.exists() {
                return Ok(Fetched::Done(path));
            }
            // 写完的文件被外面删了（系统清理、备份还原）：当没下过，从头再下。
            record.file = None;
            record.etag = None;
            record.total = None;
        }
        std::fs::create_dir_all(&self.dir).map_err(|e| local(&e))?;
        let part = self.part(task);
        let have = std::fs::metadata(&part).map_or(0, |m| m.len());
        let resume = match (&record.etag, may_resume && have > 0) {
            (Some(etag), true) => Some((have, etag.clone())),
            _ => None,
        };
        let mut request = api.request(Method::GET, &task_item_content_path(task, &item.id));
        if let Some((have, etag)) = &resume {
            request = request
                .header(RANGE, format!("bytes={have}-"))
                .header(IF_RANGE, etag);
        }
        let response = tokio::time::timeout(STALL, request.send())
            .await
            .map_err(|_| stalled())?
            .map_err(|e| unreachable(&e))?;
        if response.status() == StatusCode::RANGE_NOT_SATISFIABLE && resume.is_some() {
            return self.restart(&part, &mut record);
        }
        let response = checked(response).await?;
        let range = header(&response, CONTENT_RANGE).and_then(|v| content_range(&v));
        let partial = response.status() == StatusCode::PARTIAL_CONTENT;
        // 没要 Range 时源也可以回 206，只要从 0 起就是整份。
        let expected = resume.as_ref().map_or(0, |(have, _)| *have);
        let start = match (partial, range) {
            (false, _) => 0,
            (true, Some((start, _))) if start == expected => start,
            (true, _) => return self.restart(&part, &mut record),
        };
        if start == 0 {
            // 先清空半截再记新源的 ETag：记录里的 ETag 必须描述半截里的字节，清不掉就别换。
            std::fs::File::create(&part).map_err(|e| local(&e))?;
        }
        record.etag = header(&response, ETAG);
        record.total = match range {
            Some((_, total)) if partial => total,
            _ => response.content_length(),
        };
        self.write_record(&record)?;
        let ext = extension(
            header(&response, CONTENT_TYPE).as_deref(),
            header(&response, CONTENT_DISPOSITION).as_deref(),
        );
        let done = receive(api, task, &part, start, record.total, response).await?;
        if record.total.is_some_and(|total| total != done) {
            return Err(failure(
                MediaFailureCode::TransportUnavailable,
                text("video-download-incomplete", &[]),
            ));
        }
        let name = format!("{FILES}/{task}.{ext}");
        let target = self.dir.join(&name);
        std::fs::create_dir_all(self.dir.join(FILES)).map_err(|e| local(&e))?;
        tokio::fs::rename(&part, &target)
            .await
            .map_err(|e| local(&e))?;
        record.file = Some(name);
        record.total = Some(done);
        self.write_record(&record)?;
        Ok(Fetched::Done(target))
    }

    /// 半截接不上：丢掉，忘掉 ETag，让下一次从 0 取。
    fn restart(&self, part: &Path, record: &mut Record) -> Result<Fetched, MediaFailure> {
        match std::fs::remove_file(part) {
            Ok(()) => {}
            Err(e) if e.kind() == ErrorKind::NotFound => {}
            Err(e) => return Err(local(&e)),
        }
        record.etag = None;
        record.total = None;
        self.write_record(record)?;
        Ok(Fetched::Restart)
    }

    /// 这一条在这台设备上的离线副本；没下过为空。下过几次时取写完的那份。
    pub(crate) fn offline(&self, item_id: &str) -> Option<OfflineState> {
        let mut downloading = None;
        for record in self.records() {
            if record.item.id != item_id {
                continue;
            }
            if let Some(file) = &record.file {
                let path = self.dir.join(file);
                if path.exists() {
                    return Some(OfflineState::Ready {
                        path: path.to_string_lossy().into_owned(),
                    });
                }
                continue;
            }
            let done = std::fs::metadata(self.part(record.task)).map_or(0, |m| m.len());
            downloading = Some(OfflineState::Downloading {
                done,
                total: record.total,
            });
        }
        downloading
    }

    pub(crate) fn records(&self) -> Vec<Record> {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        entries
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
            .filter_map(|e| std::fs::read(e.path()).ok())
            .filter_map(|bytes| serde_json::from_slice(&bytes).ok())
            .collect()
    }

    /// 删掉一条的离线副本（写完的文件、半截和记录）。在下的先停。
    pub(crate) fn remove(&self, item_id: &str) -> Result<(), MediaFailure> {
        for record in self.records().into_iter().filter(|r| r.item.id == item_id) {
            self.stop(record.task);
            // 先删写完的文件再删记录：删到一半失败时记录还在，下次还找得到它。
            let mut paths: Vec<PathBuf> = record
                .file
                .map(|file| self.dir.join(file))
                .into_iter()
                .collect();
            paths.extend([self.part(record.task), self.record_path(record.task)]);
            for path in paths {
                match std::fs::remove_file(&path) {
                    Ok(()) => {}
                    Err(e) if e.kind() == ErrorKind::NotFound => {}
                    Err(e) => return Err(local(&e)),
                }
            }
        }
        Ok(())
    }
}

/// 把回体写进半截文件，边写边节流报进度；返回半截现在多长。
async fn receive(
    api: &MediaApi,
    task: TaskId,
    part: &Path,
    start: u64,
    total: Option<u64>,
    response: Response,
) -> Result<u64, MediaFailure> {
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(start > 0)
        .truncate(start == 0)
        .open(part)
        .await
        .map_err(|e| local(&e))?;
    let mut done = start;
    let mut last = Instant::now();
    let mut body = response.bytes_stream();
    while let Some(chunk) = tokio::time::timeout(STALL, body.next())
        .await
        .map_err(|_| stalled())?
    {
        let chunk = chunk.map_err(|e| {
            failure(
                MediaFailureCode::TransportUnavailable,
                text(
                    "video-download-interrupted",
                    &[("reason", e.to_string().into())],
                ),
            )
        })?;
        file.write_all(&chunk).await.map_err(|e| local(&e))?;
        done += chunk.len() as u64;
        if last.elapsed() >= REPORT_EVERY {
            last = Instant::now();
            // 进度报不上不算失败：完成时核心按满算。
            let report = TaskProgressReport { done, total };
            let _ = api.post_empty(&task_progress_path(task), &report).await;
        }
    }
    file.flush().await.map_err(|e| local(&e))?;
    file.sync_all().await.map_err(|e| local(&e))?;
    Ok(done)
}

fn stalled() -> MediaFailure {
    failure(
        MediaFailureCode::TransportUnavailable,
        text(
            "video-download-stalled",
            &[("seconds", STALL.as_secs().into())],
        ),
    )
}

fn header(response: &Response, name: reqwest::header::HeaderName) -> Option<String> {
    response
        .headers()
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
}

/// `bytes 100-199/1000` 的起点和总长；总长是 `*` 时为空。
fn content_range(value: &str) -> Option<(u64, Option<u64>)> {
    let (span, total) = value.strip_prefix("bytes ")?.split_once('/')?;
    let start = span.split_once('-')?.0.trim().parse().ok()?;
    Some((start, total.trim().parse().ok()))
}

/// 文件的扩展名：先看类型，再看提供方给的文件名，都认不出用 `bin`。播放器靠扩展名认容器。
fn extension(content_type: Option<&str>, disposition: Option<&str>) -> String {
    let mime = content_type
        .and_then(|t| t.split(';').next())
        .map(|t| t.trim().to_ascii_lowercase());
    let by_type = match mime.as_deref() {
        Some("video/mp4") => Some("mp4"),
        Some("video/x-matroska" | "video/matroska") => Some("mkv"),
        Some("video/quicktime") => Some("mov"),
        Some("video/webm") => Some("webm"),
        Some("video/x-msvideo") => Some("avi"),
        Some("video/mp2t") => Some("ts"),
        Some("audio/mpeg") => Some("mp3"),
        Some("audio/flac") => Some("flac"),
        Some("audio/mp4") => Some("m4a"),
        _ => None,
    };
    if let Some(ext) = by_type {
        return ext.to_owned();
    }
    disposition
        .and_then(|d| d.split("filename=").nth(1))
        .map(|name| {
            name.split(';')
                .next()
                .unwrap_or(name)
                .trim()
                .trim_matches('"')
        })
        .and_then(|name| {
            name.rsplit_once('.')
                .map(|(_, ext)| ext.to_ascii_lowercase())
        })
        .filter(|ext| {
            !ext.is_empty() && ext.len() <= 5 && ext.chars().all(|c| c.is_ascii_alphanumeric())
        })
        .unwrap_or_else(|| "bin".to_owned())
}

/// 本机读写的错：空间不足单列，壳要让人去清手机。
fn local(error: &std::io::Error) -> MediaFailure {
    let code = match error.kind() {
        ErrorKind::StorageFull | ErrorKind::QuotaExceeded => MediaFailureCode::LocalStorageFull,
        ErrorKind::PermissionDenied | ErrorKind::ReadOnlyFilesystem => {
            MediaFailureCode::LocalPermissionDenied
        }
        _ => MediaFailureCode::LocalIo,
    };
    failure(code, error.to_string())
}

#[cfg(test)]
#[path = "download_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "download_edge_tests.rs"]
mod edge_tests;
