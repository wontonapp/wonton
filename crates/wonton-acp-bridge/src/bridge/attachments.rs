//! 附件：把主人发来的图片和文件交给 Agent。
//!
//! 一条媒体消息一轮（多图分组收齐是下一步）。收到消息时就把附件下载、解密并落到
//! Agent 工作区（bridge 和 agent 共享的 bind mount，见 ADR-0035）；发提示词时按 Agent
//! 的 `promptCapabilities` 组块：声明了 image 的给 `image` 块（base64 + mimeType），
//! 其余一律给 `resource_link`（`file://` 工作区路径）——所有 Agent 必须支持
//! `resource_link`，且 ACP 安全指引不建议把外部 HTTP 地址交给 Agent 去取。

use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::Room;
use crate::{
    error::{Detail, Error, Result},
    matrix::MediaAttachment,
};

/// 单个附件的字节数上限：20 MiB。
///
/// 手机照片原图 1–8 MiB、截图更小，聊天里超过这个数的多半是录屏或大文档，那类内容
/// 本来就该走文件服务（`wonton://file`，另行设计）。20 MiB 经 base64 放大 4/3 约
/// 27 MiB，加上提示词仍装得下一帧 JSON（见 [`crate::config::MAX_FRAME_BYTES`]）。
pub(crate) const MAX_ATTACHMENT_BYTES: usize = 20 * 1024 * 1024;

/// 附件在工作区里的保留期：7 天。
const RETENTION: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// 附件目录的总大小上限：1 GiB，超了从最旧的开始删。
const MAX_TOTAL_BYTES: u64 = 1024 * 1024 * 1024;

/// 附件目录：固定挂在工作区根下，清理只动这里。
pub(crate) const ROOT: &str = "wonton-attachments";

/// 每小时清一次；启动时也会清一次。
pub(crate) const SWEEP_INTERVAL: Duration = Duration::from_secs(60 * 60);

/// 已落工作区的一份附件。进排队（话题准入队列会经 topics.json 落盘）时只带这些
/// 没有敏感信息的字段；媒体句柄在下载之后就不需要了。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PreparedAttachment {
    pub(crate) path: PathBuf,
    pub(crate) name: String,
    pub(crate) mime: String,
    pub(crate) size: u64,
}

impl PreparedAttachment {
    pub(crate) fn is_image(&self) -> bool {
        self.mime.starts_with("image/")
    }
}

/// 收到媒体消息就下载、解密、落工作区；失败按类别报给主人，不开轮。
pub(super) async fn prepare<R: Room>(
    room: &R,
    media: &MediaAttachment,
    event_id: &str,
    workspace: &str,
) -> Result<PreparedAttachment> {
    if let Some(declared) = media.declared_size
        && declared > MAX_ATTACHMENT_BYTES as u64
    {
        return Err(too_large());
    }
    let bytes = room
        .download_media(&media.token)
        .await
        .map_err(|error| Error::Matrix(Detail::data(error.to_string())))?;
    if bytes.len() > MAX_ATTACHMENT_BYTES {
        return Err(too_large());
    }
    let mime = media
        .mime_type
        .clone()
        .unwrap_or_else(|| "application/octet-stream".to_owned());
    let path = save(room.room_id().as_str(), event_id, workspace, media, &bytes).await?;
    Ok(PreparedAttachment {
        name: path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_owned(),
        path,
        mime,
        size: bytes.len() as u64,
    })
}

/// 按能力把已落盘的附件组内容块。
pub(super) async fn blocks(
    prepared: &PreparedAttachment,
    agent_takes_images: bool,
    workspace: &str,
) -> Result<Vec<Value>> {
    if agent_takes_images && prepared.is_image() {
        let bytes = tokio::fs::read(&prepared.path).await?;
        let data = base64::engine::general_purpose::STANDARD.encode(bytes);
        return Ok(vec![json!({
            "type": "image",
            "data": data,
            "mimeType": prepared.mime,
        })]);
    }
    let uri = url::Url::from_file_path(&prepared.path)
        .map_err(|_| {
            Error::InvalidInput(Detail::msg_with(
                "workspace-not-absolute",
                [("path", workspace.to_owned())],
            ))
        })?
        .to_string();
    Ok(vec![json!({
        "type": "resource_link",
        "uri": uri,
        "name": prepared.name,
        "mimeType": prepared.mime,
        "size": prepared.size,
    })])
}

pub(super) fn text_block(text: &str) -> Value {
    json!({"type": "text", "text": text})
}

fn too_large() -> Error {
    Error::InvalidInput(Detail::msg_with(
        "attachment-too-large",
        [("max", format_mb(MAX_ATTACHMENT_BYTES))],
    ))
}

/// 20 MiB 这类数在提示里写成 “20 MB”：向上取整，不出现 10485760。
fn format_mb(bytes: usize) -> String {
    format!("{} MB", bytes.div_ceil(1024 * 1024))
}

/// 落盘：`{workspace}/wonton-attachments/{房间}/{事件}/{文件名}`。工作区必须已存在
/// （bind mount 没挂上是插件错误，不悄悄创建一个 Agent 看不见的目录）。
async fn save(
    room_id: &str,
    event_id: &str,
    workspace: &str,
    media: &MediaAttachment,
    bytes: &[u8],
) -> Result<PathBuf> {
    let root = Path::new(workspace);
    if !root.is_dir() {
        return Err(Error::AgentNotReady(Detail::msg("workspace-missing")));
    }
    let dir = root
        .join(ROOT)
        .join(sanitize(room_id))
        .join(sanitize(event_id));
    let file = dir.join(sanitize_filename(&media.filename));
    tokio::fs::create_dir_all(&dir).await?;
    tokio::fs::write(&file, bytes).await?;
    // agent 容器可能以别的用户跑，至少让所有人可读。
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = tokio::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).await;
    }
    Ok(file)
}

/// 房间和事件 ID 只保留安全字符做目录名，其余换成 `_`。
fn sanitize(id: &str) -> String {
    let cleaned: String = id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.is_empty() {
        "unknown".to_owned()
    } else {
        cleaned
    }
}

/// 文件名清洗：只留最后一段路径，去掉控制字符，掐头去尾的点和空白，超长截短；
/// 清洗完为空就用兜底名。防 `..` 和绝对路径写进工作区。
fn sanitize_filename(name: &str) -> String {
    let last = name.rsplit(['/', '\\']).next().unwrap_or("");
    let cleaned: String = last.chars().filter(|c| !c.is_control()).collect();
    let trimmed = cleaned.trim().trim_matches('.');
    let mut name: String = trimmed.chars().take(200).collect();
    if name.is_empty() {
        name = "attachment".to_owned();
    }
    name
}

/// 清理工作区附件目录：删超过保留期的，总量超上限时从最旧的删起，顺手删空目录。
/// 只动 [`ROOT`] 下；失败只记日志（清理是尽力而为，不挡消息）。
pub(crate) async fn sweep(workspace: &str) {
    let root = Path::new(workspace).join(ROOT);
    let Ok(rooms) = std::fs::read_dir(&root) else {
        return;
    };
    let now = std::time::SystemTime::now();
    let mut files: Vec<(PathBuf, std::time::SystemTime, u64)> = Vec::new();
    for room in rooms.flatten() {
        let Ok(events) = std::fs::read_dir(room.path()) else {
            continue;
        };
        for event in events.flatten() {
            let Ok(attachments) = std::fs::read_dir(event.path()) else {
                continue;
            };
            for file in attachments.flatten() {
                let Ok(metadata) = file.metadata() else {
                    continue;
                };
                if metadata.is_file() {
                    files.push((file.path(), modified(&metadata, now), metadata.len()));
                }
            }
        }
    }
    let mut total: u64 = files.iter().map(|(_, _, size)| size).sum();
    // 从最旧的开始：过期先走，超上限也先删旧的。
    files.sort_by_key(|(_, modified, _)| *modified);
    for (path, modified, size) in &files {
        let expired = now
            .duration_since(*modified)
            .is_ok_and(|age| age > RETENTION);
        if !expired && total <= MAX_TOTAL_BYTES {
            continue;
        }
        if std::fs::remove_file(path).is_ok() {
            total = total.saturating_sub(*size);
        } else {
            tracing::warn!(path = %path.display(), "删不掉过期附件");
        }
    }
    for room in std::fs::read_dir(&root).into_iter().flatten().flatten() {
        for event in std::fs::read_dir(room.path())
            .into_iter()
            .flatten()
            .flatten()
        {
            let _ = std::fs::remove_dir(event.path());
        }
        let _ = std::fs::remove_dir(room.path());
    }
}

fn modified(metadata: &std::fs::Metadata, now: std::time::SystemTime) -> std::time::SystemTime {
    metadata.modified().unwrap_or(now)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filenames_lose_paths_dots_and_controls() {
        assert_eq!(sanitize_filename("报告 v2.pdf"), "报告 v2.pdf");
        // 只留最后一段路径，前面的目录层级全丢。
        assert_eq!(sanitize_filename("../../../etc/passwd"), "passwd");
        assert_eq!(sanitize_filename("..\\..\\win.ini"), "win.ini");
        assert_eq!(sanitize_filename(".."), "attachment");
        assert_eq!(sanitize_filename(""), "attachment");
        assert_eq!(sanitize_filename("a\u{0}b"), "ab");
        let long = sanitize_filename(&"长".repeat(300));
        assert_eq!(long.chars().count(), 200);
    }

    #[test]
    fn ids_keep_safe_characters_only() {
        assert_eq!(sanitize("!room:example.org"), "_room_example.org");
        assert_eq!(sanitize("$evt.1-2_x"), "_evt.1-2_x");
        assert_eq!(sanitize(""), "unknown");
    }

    #[tokio::test]
    async fn sweep_removes_expired_and_oversized_but_keeps_fresh() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join(ROOT);
        std::fs::create_dir_all(root.join("r/e1")).unwrap();
        std::fs::create_dir_all(root.join("r/e2")).unwrap();
        std::fs::write(root.join("r/e1/old.bin"), vec![0; 10]).unwrap();
        std::fs::write(root.join("r/e2/new.bin"), vec![0; 10]).unwrap();
        let old = root.join("r/e1/old.bin");
        let fresh = root.join("r/e2/new.bin");
        let expired = std::time::SystemTime::now() - RETENTION - Duration::from_secs(60);
        set_mtime(&old, expired);

        sweep(dir.path().to_str().unwrap()).await;
        assert!(!old.exists(), "过期附件应被删");
        assert!(fresh.exists(), "新附件应保留");
        assert!(!root.join("r/e1").exists(), "空目录应被删");
    }

    /// std 没有直接改 mtime 的函数；重开文件用 `set_times` 写。
    fn set_mtime(path: &Path, time: std::time::SystemTime) {
        use std::fs::FileTimes;
        let file = std::fs::OpenOptions::new().append(true).open(path).unwrap();
        file.set_times(FileTimes::new().set_modified(time)).unwrap();
    }

    #[tokio::test]
    async fn sweep_enforces_the_total_size_cap() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join(ROOT);
        std::fs::create_dir_all(root.join("r/e1")).unwrap();
        std::fs::create_dir_all(root.join("r/e2")).unwrap();
        // 稀疏文件：只在元数据上占 1 GiB，不真写盘。
        let big = root.join("r/e1/big.bin");
        let file = std::fs::File::create(&big).unwrap();
        file.set_len(MAX_TOTAL_BYTES).unwrap();
        drop(file);
        // 大文件更旧：超上限从最旧的删起，删掉它就回到限额以内，新的留下。
        set_mtime(&big, std::time::SystemTime::now() - Duration::from_secs(60));
        let small = root.join("r/e2/small.bin");
        std::fs::write(&small, vec![0; 10]).unwrap();
        sweep(dir.path().to_str().unwrap()).await;
        assert!(!big.exists(), "超上限时最旧的应先被删");
        assert!(small.exists(), "删到限额以内就停");
    }
}
