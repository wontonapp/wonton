//! 下载（ADR-0020）。文件放进 App 自己的目录，记录里只存相对路径。先写 `<目标>.part`，传完才改名，目标路径上出现的总是完整的文件。接着下时带
//! `Range` 和 `If-Range`：文件变了核心给整个文件，从头来。

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use reqwest::{Method, StatusCode};
use tokio::io::AsyncWriteExt;
use wonton_protocol::{FileFailure, FileId, FileInfo};

use super::FilesLink;
use super::api::{Api, Problem, checked, header, interrupted, pausable};
use super::store::{Job, Record};

/// 这么久收不到一块就算卡住了。
const STALL: Duration = Duration::from_secs(60);

pub(super) fn part_path(destination: &Path) -> PathBuf {
    let mut part = destination.as_os_str().to_owned();
    part.push(".part");
    PathBuf::from(part)
}

/// 这个文件在下载目录里的相对路径。一个编号一个目录：不同的文件可以同名，同一个文件再下载就覆盖。
pub(super) fn relative_path(id: &FileId, name: &str) -> String {
    format!("{id}/{}", local_file_name(name, id))
}

/// 名字来自核心，Bot 也能影响它，手机上不能照着它写到别的目录。留出 `.part` 后缀的位置：文件系统限制一段 255 字节。
fn local_file_name(name: &str, id: &FileId) -> String {
    const MAX_BYTES: usize = 240;
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    let cleaned = cleaned.trim().trim_start_matches('.');
    if cleaned.is_empty() {
        return id.as_str().to_owned();
    }
    if cleaned.len() <= MAX_BYTES {
        return cleaned.to_owned();
    }
    // 截短时留下扩展名：系统靠它选打开的 App。
    let extension = Path::new(cleaned)
        .extension()
        .and_then(|extension| extension.to_str())
        .filter(|extension| extension.len() <= 16)
        .map_or(String::new(), |extension| format!(".{extension}"));
    let mut end = MAX_BYTES - extension.len();
    while !cleaned.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{extension}", &cleaned[..end])
}

fn local(error: &io::Error) -> Problem {
    let reason = match error.kind() {
        io::ErrorKind::StorageFull | io::ErrorKind::QuotaExceeded => FileFailure::LocalStorageFull,
        io::ErrorKind::PermissionDenied | io::ErrorKind::ReadOnlyFilesystem => {
            FileFailure::LocalPermissionDenied
        }
        _ => FileFailure::LocalIo,
    };
    Problem::Failed(reason, format!("写不了下载的文件：{error}"))
}

pub(super) async fn run(link: &Arc<FilesLink>, record: Record) -> Result<(), Problem> {
    let Job::Download { path, etag } = record.job else {
        unreachable!("下载的记录才走这里");
    };
    let file = record.file.expect("下载的记录一开始就有编号");
    let api = Api::connect(&link.home)?;
    let path = match path {
        Some(path) => path,
        None => place(link, &api, &record.id, &file).await?,
    };
    let destination = link.downloads_dir.join(path);
    let part = part_path(&destination);
    let offset = prepare(&part, etag.is_some()).await?;
    link.store.progress(&record.id, offset);
    let mut request = api.stream(Method::GET, &Api::content_path(&file));
    if offset > 0
        && let Some(etag) = &etag
    {
        request = request
            .header(reqwest::header::RANGE, format!("bytes={offset}-"))
            .header(reqwest::header::IF_RANGE, etag);
    }
    let response = tokio::time::timeout(STALL, request.send())
        .await
        .map_err(|_| Problem::Interrupted("核心一直没有回答".to_owned()))?
        .map_err(interrupted)?;
    if response.status() == StatusCode::RANGE_NOT_SATISFIABLE {
        // 半截文件比核心那边的文件还长：文件变了。
        reset(&part).await?;
        return Err(Problem::Interrupted("文件变短了，从头下".to_owned()));
    }
    let response = checked(response).await.map_err(pausable)?;
    let resumed = response.status() == StatusCode::PARTIAL_CONTENT;
    let start = if resumed {
        match content_range(header(response.headers(), "content-range")) {
            Some((start, _)) if start == offset => start,
            _ => {
                reset(&part).await?;
                return Err(Problem::Interrupted(
                    "核心给的范围对不上，从头下".to_owned(),
                ));
            }
        }
    } else {
        reset(&part).await?;
        0
    };
    let total = if resumed {
        content_range(header(response.headers(), "content-range")).and_then(|(_, total)| total)
    } else {
        response.content_length()
    };
    let new_etag = header(response.headers(), "etag").map(str::to_owned);
    link.store.update(&record.id, |record| {
        record.total_bytes = total;
        record.transferred_bytes = start;
        if let Job::Download { etag, .. } = &mut record.job {
            *etag = new_etag;
        }
    });
    let received = receive(link, &record.id, &part, start, response).await?;
    if total.is_some_and(|total| total != received) {
        return Err(Problem::Interrupted("连接断了，没收全".to_owned()));
    }
    tokio::fs::rename(&part, &destination)
        .await
        .map_err(|e| local(&e))?;
    Ok(())
}

/// 问核心文件叫什么，定下放在哪。
async fn place(
    link: &Arc<FilesLink>,
    api: &Api,
    id: &str,
    file: &FileId,
) -> Result<String, Problem> {
    let info = api.info::<FileInfo>(file).await.map_err(pausable)?;
    let path = relative_path(file, &info.name);
    link.store.update(id, |record| {
        record.name = info.name;
        if let Job::Download { path: placed, .. } = &mut record.job {
            *placed = Some(path.clone());
        }
    });
    Ok(path)
}

/// 建好目录，返回半截文件有多长。没有 ETag 接不上：`If-Range` 没得比，万一文件变了就会拼出一个坏文件，所以从头下。
async fn prepare(part: &Path, resumable: bool) -> Result<u64, Problem> {
    if let Some(parent) = part.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| local(&e))?;
    }
    if !resumable {
        return Ok(0);
    }
    match tokio::fs::metadata(part).await {
        Ok(meta) => Ok(meta.len()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(0),
        Err(e) => Err(local(&e)),
    }
}

async fn reset(part: &Path) -> Result<(), Problem> {
    match tokio::fs::remove_file(part).await {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(local(&e)),
    }
}

/// `bytes <起点>-<终点>/<总长>` 的起点和总长。
fn content_range(value: Option<&str>) -> Option<(u64, Option<u64>)> {
    let (range, total) = value?.strip_prefix("bytes ")?.split_once('/')?;
    let start = range.split_once('-')?.0.parse().ok()?;
    Some((start, total.parse().ok()))
}

async fn receive(
    link: &Arc<FilesLink>,
    id: &str,
    part: &Path,
    start: u64,
    response: reqwest::Response,
) -> Result<u64, Problem> {
    let mut output = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(part)
        .await
        .map_err(|e| local(&e))?;
    let mut received = start;
    let mut body = response.bytes_stream();
    loop {
        let chunk = tokio::time::timeout(STALL, body.next())
            .await
            .map_err(|_| Problem::Interrupted("收不到数据，隧道可能卡住了".to_owned()))?;
        let Some(chunk) = chunk else {
            break;
        };
        let chunk = chunk.map_err(interrupted)?;
        output.write_all(&chunk).await.map_err(|e| local(&e))?;
        received += chunk.len() as u64;
        link.store.progress(id, received);
    }
    output.flush().await.map_err(|e| local(&e))?;
    output.sync_all().await.map_err(|e| local(&e))?;
    Ok(received)
}
