//! 收件目录的文件操作：认领（校验清单、整个目录改名搬走）和过期清理。格式见 [`wonton_protocol::ShareInManifest`]。

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use wonton_l10n::l10n;
use wonton_protocol::{
    AttachmentKind, CoreError, SHARE_INBOX_DIR, SHARE_MANIFEST_FILE, SHARE_MAX_ITEMS,
    SHARE_READY_FILE, ShareInManifest, ShareInManifestError, is_share_token,
};

use crate::l10n::{CATALOG, language};

/// 认领后的目录放在这里，和收件目录同在数据目录下，改名是一次 `rename`。
pub(super) const CLAIMED_DIR: &str = "share-claimed";
/// 没被认领、没发出去的目录留多久：和文件服务清掉没传完的上传同一个期限（ADR-0020）。
pub(super) const KEEP_FOR: Duration = Duration::from_secs(24 * 60 * 60);

/// 认领下来的一次分享。
#[derive(Clone, Debug)]
pub(crate) struct Claimed {
    pub(super) token: String,
    pub(super) dir: PathBuf,
    pub(super) text: String,
    pub(super) items: Vec<ClaimedItem>,
    pub(super) dropped: u32,
}

#[derive(Clone, Debug)]
pub(crate) struct ClaimedItem {
    /// 清单里的文件名，在这次分享里唯一。
    pub(super) id: String,
    pub(super) kind: AttachmentKind,
    pub(super) name: String,
    pub(super) mime_type: Option<String>,
    pub(super) size: u64,
    pub(super) path: PathBuf,
}

fn invalid(reason: String) -> CoreError {
    CoreError::InvalidInput { reason }
}

fn storage(path: &Path, error: &io::Error) -> CoreError {
    CoreError::Storage {
        reason: l10n!(
            CATALOG,
            language(),
            "share-in-store-failed",
            path = path.display().to_string(),
            reason = error.to_string()
        ),
    }
}

/// 把 `<root>/share-inbox/<token>` 搬到 `<root>/share-claimed/<token>` 并读出清单。同一个令牌只能认领一次：
/// 认领过的目录已经不在收件目录里，重放的链接得到“找不到”。没写完（没有 `.ready`）、清单不合规则、列出的文件不在的目录
/// 删掉，返回 `InvalidInput`。
pub(super) fn claim(root: &Path, token: &str) -> Result<Claimed, CoreError> {
    if !is_share_token(token) {
        return Err(invalid(l10n!(CATALOG, language(), "share-in-not-found")));
    }
    let inbox = root.join(SHARE_INBOX_DIR).join(token);
    if !inbox.is_dir() {
        return Err(invalid(l10n!(CATALOG, language(), "share-in-not-found")));
    }
    if !inbox.join(SHARE_READY_FILE).is_file() {
        remove(&inbox);
        return Err(invalid(l10n!(CATALOG, language(), "share-in-incomplete")));
    }
    let claimed_root = root.join(CLAIMED_DIR);
    fs::create_dir_all(&claimed_root).map_err(|error| storage(&claimed_root, &error))?;
    let dir = claimed_root.join(token);
    if dir.exists() {
        remove(&dir);
    }
    fs::rename(&inbox, &dir).map_err(|error| storage(&dir, &error))?;
    read(token, dir.clone()).inspect_err(|_| remove(&dir))
}

fn read(token: &str, dir: PathBuf) -> Result<Claimed, CoreError> {
    let unreadable = |reason: String| {
        invalid(l10n!(
            CATALOG,
            language(),
            "share-in-unreadable",
            reason = reason
        ))
    };
    let bytes =
        fs::read(dir.join(SHARE_MANIFEST_FILE)).map_err(|error| unreadable(error.to_string()))?;
    let manifest = ShareInManifest::parse(&bytes).map_err(|error| match error {
        ShareInManifestError::Empty => invalid(l10n!(CATALOG, language(), "share-in-empty")),
        other => unreadable(other.to_string()),
    })?;
    let max = usize::try_from(SHARE_MAX_ITEMS).expect("a small constant");
    let dropped = manifest.items.len().saturating_sub(max);
    let mut items = Vec::with_capacity(manifest.items.len().min(max));
    for entry in manifest.items.into_iter().take(max) {
        let path = dir.join(&entry.file);
        let metadata = fs::metadata(&path)
            .map_err(|_| invalid(l10n!(CATALOG, language(), "share-in-incomplete")))?;
        if !metadata.is_file() {
            return Err(invalid(l10n!(CATALOG, language(), "share-in-incomplete")));
        }
        let name = if entry.name.trim().is_empty() {
            entry.file.clone()
        } else {
            entry.name.clone()
        };
        items.push(ClaimedItem {
            kind: entry.kind(),
            id: entry.file,
            name,
            mime_type: entry.mime,
            size: metadata.len(),
            path,
        });
    }
    Ok(Claimed {
        token: token.to_owned(),
        dir,
        text: manifest.text.unwrap_or_default().trim().to_owned(),
        items,
        dropped: u32::try_from(dropped).unwrap_or(u32::MAX),
    })
}

/// 删掉收件目录和认领目录里放了超过 [`KEEP_FOR`] 的分享：没被认领的（App 没打开过）、认领了没发的、发出去后留着给文件续传的。
/// 还在写的分享（扩展正在复制）不会这么旧，不受影响。
pub(super) fn clean(root: &Path, now: SystemTime) {
    for parent in [root.join(SHARE_INBOX_DIR), root.join(CLAIMED_DIR)] {
        let Ok(entries) = fs::read_dir(&parent) else {
            continue;
        };
        for entry in entries.flatten() {
            let modified = entry.metadata().and_then(|metadata| metadata.modified());
            let expired = modified
                .is_ok_and(|modified| now.duration_since(modified).is_ok_and(|age| age > KEEP_FOR));
            if expired {
                remove(&entry.path());
            }
        }
    }
}

/// 删不掉只记日志：下次启动再清。
pub(super) fn remove(path: &Path) {
    let result = if path.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    };
    if let Err(error) = result
        && error.kind() != io::ErrorKind::NotFound
    {
        tracing::warn!(%error, path = %path.display(), "could not remove a shared item");
    }
}
