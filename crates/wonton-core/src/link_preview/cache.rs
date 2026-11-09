//! 按网址缓存预览（ADR-0065）：取到的留 24 小时，失败的留 10 分钟，图片和预览存在一起。
//!
//! 一个网址一个 `<键>.json`，有图片时旁边是 `<键>.img`；键是网址的 SHA-256，也是图片的标识。图片先写、记录后写，
//! 记录在就说明图片已经写完。

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use wonton_protocol::{LinkPreview, LinkPreviewFailure};

use super::{Clock, Failure};

const PREVIEW_TTL_MS: u64 = 24 * 60 * 60 * 1000;
const FAILURE_TTL_MS: u64 = 10 * 60 * 1000;
/// 清理过期条目的间隔：每次写都扫一遍目录太费。
const SWEEP_EVERY_MS: u64 = 10 * 60 * 1000;
/// 缓存目录的总大小上限，超了从最旧的删起。数值待实测（ADR-0065“代价与待验证项”）。
const MAX_BYTES: u64 = 512 << 20;

#[derive(Serialize, Deserialize)]
struct Entry {
    stored_at_ms: u64,
    outcome: Outcome,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Outcome {
    Preview(LinkPreview),
    Failed {
        kind: LinkPreviewFailure,
        detail: String,
    },
}

/// 缓存的图片。
pub(crate) struct Image {
    pub mime_type: String,
    pub bytes: Vec<u8>,
}

pub(super) struct Cache {
    dir: PathBuf,
    now: Clock,
    last_sweep_ms: Mutex<u64>,
}

/// 网址的缓存键，也是它的图片标识。
pub(super) fn key(url: &str) -> String {
    Sha256::digest(url.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

impl Cache {
    pub fn new(dir: PathBuf, now: Clock) -> Self {
        Self {
            dir,
            now,
            last_sweep_ms: Mutex::new(0),
        }
    }

    /// 还没过期的结果。读不出来的记录当作没有缓存，重新取。
    pub fn get(&self, key: &str) -> Option<Result<LinkPreview, Failure>> {
        let entry = self.entry(key)?;
        match entry.outcome {
            Outcome::Preview(preview) => Some(Ok(preview)),
            Outcome::Failed { kind, detail } => Some(Err(Failure::new(kind, detail))),
        }
    }

    /// 还没过期的预览图片。`id` 不是我们发出去的那种键就当没有，不拿它拼路径。
    pub fn image(&self, id: &str) -> Option<Image> {
        if id.len() != 64 || !id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
            return None;
        }
        let Outcome::Preview(preview) = self.entry(id)?.outcome else {
            return None;
        };
        let image = preview.image?;
        let bytes = fs::read(self.image_path(id)).ok()?;
        Some(Image {
            mime_type: image.mime_type,
            bytes,
        })
    }

    fn entry(&self, key: &str) -> Option<Entry> {
        let text = fs::read(self.entry_path(key)).ok()?;
        let entry: Entry = serde_json::from_slice(&text).ok()?;
        (!self.expired(&entry)).then_some(entry)
    }

    fn expired(&self, entry: &Entry) -> bool {
        let ttl = match entry.outcome {
            Outcome::Preview(_) => PREVIEW_TTL_MS,
            Outcome::Failed { .. } => FAILURE_TTL_MS,
        };
        (self.now)().saturating_sub(entry.stored_at_ms) >= ttl
    }

    /// 记下结果。写不进去只记日志：缓存是省事用的，取到的结果照样回给设备。
    pub fn put(&self, key: &str, result: &Result<LinkPreview, Failure>, image: Option<&[u8]>) {
        let outcome = match result {
            Ok(preview) => Outcome::Preview(preview.clone()),
            Err(failure) => Outcome::Failed {
                kind: failure.kind,
                detail: failure.detail.clone(),
            },
        };
        let entry = Entry {
            stored_at_ms: (self.now)(),
            outcome,
        };
        if let Err(e) = self.write(key, &entry, image) {
            tracing::warn!(error = %e, dir = %self.dir.display(), "链接预览写不进缓存");
        }
        self.sweep_now_and_then();
    }

    fn write(&self, key: &str, entry: &Entry, image: Option<&[u8]>) -> io::Result<()> {
        fs::create_dir_all(&self.dir)?;
        match image {
            Some(bytes) => write_atomic(&self.image_path(key), bytes)?,
            None => remove_if_present(&self.image_path(key))?,
        }
        let json = serde_json::to_vec(entry).map_err(io::Error::other)?;
        write_atomic(&self.entry_path(key), &json)
    }

    fn sweep_now_and_then(&self) {
        let now = (self.now)();
        {
            let mut last = self.last_sweep_ms.lock().expect("缓存清理锁中毒");
            if now.saturating_sub(*last) < SWEEP_EVERY_MS {
                return;
            }
            *last = now;
        }
        if let Err(e) = self.sweep() {
            tracing::warn!(error = %e, dir = %self.dir.display(), "清理链接预览缓存失败");
        }
    }

    /// 删掉过期的，总大小还超上限就从最旧的删起。
    fn sweep(&self) -> io::Result<()> {
        let mut kept = Vec::new();
        for item in fs::read_dir(&self.dir)? {
            let path = item?.path();
            let Some(key) = entry_key(&path) else {
                continue;
            };
            let entry = fs::read(&path)
                .ok()
                .and_then(|text| serde_json::from_slice::<Entry>(&text).ok());
            match entry {
                Some(entry) if !self.expired(&entry) => {
                    let size = file_size(&path) + file_size(&self.image_path(&key));
                    kept.push((entry.stored_at_ms, key, size));
                }
                _ => self.remove(&key)?,
            }
        }
        kept.sort();
        let mut total: u64 = kept.iter().map(|(_, _, size)| size).sum();
        for (_, key, size) in kept {
            if total <= MAX_BYTES {
                break;
            }
            self.remove(&key)?;
            total -= size;
        }
        Ok(())
    }

    fn remove(&self, key: &str) -> io::Result<()> {
        remove_if_present(&self.entry_path(key))?;
        remove_if_present(&self.image_path(key))
    }

    fn entry_path(&self, key: &str) -> PathBuf {
        self.dir.join(format!("{key}.json"))
    }

    fn image_path(&self, key: &str) -> PathBuf {
        self.dir.join(format!("{key}.img"))
    }
}

fn entry_key(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_str()?;
    name.strip_suffix(".json").map(str::to_owned)
}

fn file_size(path: &Path) -> u64 {
    fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

/// 先写到同目录的临时文件再改名：读的人看不到写了一半的文件。同一个网址可能有两个请求同时在写，临时文件名各不相同。
fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    let partial = path.with_file_name(format!(
        "{name}.{}.partial",
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::write(&partial, bytes)?;
    fs::rename(&partial, path)
}

fn remove_if_present(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}
