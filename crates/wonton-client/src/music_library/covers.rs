//! 封面缓存：按插件、封面 id 和边长档存在 App 的缓存目录里，系统清缓存就没了，下次再取。
//!
//! 边长取到档位上（向上取整），同一张封面在列表和大图里各存一份，不按壳要的每个像素数各存一份。
//! 总量超过上限时按修改时间删最旧的，删到上限的四分之三。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use super::api::Endpoint;
use super::fetch;

const SIZES: [u32; 4] = [160, 320, 640, 1280];
const LIMIT_BYTES: u64 = 200 << 20;
/// 每写这么多张查一次总量：每次都扫目录太费，少查几次多出来的也有限。
const PRUNE_EVERY: u32 = 64;

pub(super) struct Covers {
    dir: PathBuf,
    writes: AtomicU32,
}

fn bucket(size: u32) -> u32 {
    SIZES
        .into_iter()
        .find(|bucket| *bucket >= size)
        .unwrap_or(SIZES[SIZES.len() - 1])
}

/// 文件名里只留安全字符，其余换成 `_`，再带上原值的哈希，免得两个 id 撞成同一个文件名。
fn file_name(id: &str, size: u32) -> String {
    let safe: String = id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .take(64)
        .collect();
    let hash = id.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    });
    format!("{safe}-{hash:016x}-{size}")
}

impl Covers {
    pub(super) fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            writes: AtomicU32::new(0),
        }
    }

    pub(super) async fn get(
        &self,
        endpoint: &Endpoint,
        plugin_id: &str,
        id: &str,
        size: u32,
    ) -> Option<Vec<u8>> {
        let size = bucket(size);
        let path = self
            .dir
            .join(file_name(plugin_id, 0))
            .join(file_name(id, size));
        if let Ok(bytes) = tokio::fs::read(&path).await {
            return Some(bytes);
        }
        let bytes = match fetch::cover(endpoint, id, size).await {
            Ok(bytes) => bytes,
            Err(failure) => {
                tracing::warn!(?failure, "music cover: fetch failed");
                return None;
            }
        };
        if let Err(error) = write(&path, &bytes).await {
            tracing::warn!(%error, "music cover: cache write failed");
        } else if self.writes.fetch_add(1, Ordering::Relaxed) % PRUNE_EVERY == PRUNE_EVERY - 1 {
            let dir = self.dir.clone();
            tokio::task::spawn_blocking(move || prune(&dir, LIMIT_BYTES));
        }
        Some(bytes)
    }
}

async fn write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let parent = path.parent().expect("cover paths have a plugin directory");
    tokio::fs::create_dir_all(parent).await?;
    let partial = path.with_extension("part");
    tokio::fs::write(&partial, bytes).await?;
    tokio::fs::rename(&partial, path).await
}

/// 总量超过 `limit` 时删最旧的，删到四分之三。读不了的文件跳过。
pub(super) fn prune(dir: &Path, limit: u64) {
    let mut files: Vec<(std::time::SystemTime, u64, PathBuf)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .flat_map(|plugin| {
            std::fs::read_dir(plugin.path())
                .into_iter()
                .flatten()
                .flatten()
        })
        .filter_map(|file| {
            let meta = file.metadata().ok()?;
            Some((meta.modified().ok()?, meta.len(), file.path()))
        })
        .collect();
    let mut total: u64 = files.iter().map(|(_, len, _)| len).sum();
    if total <= limit {
        return;
    }
    files.sort_by_key(|(modified, _, _)| *modified);
    for (_, len, path) in files {
        if total <= limit / 4 * 3 {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            total -= len;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 封面 id 来自曲库，不能拿它拼出缓存目录外的路径，也不能让两个 id 落到同一个文件。
    #[test]
    fn cover_ids_cannot_escape_the_cache_or_collide() {
        assert_ne!(file_name("a/b", 160), file_name("a?b", 160));
        assert!(!file_name("../../etc", 160).contains('/'));
    }

    #[test]
    fn pruning_removes_the_oldest_until_under_three_quarters() {
        let dir = tempfile::tempdir().unwrap();
        let plugin = dir.path().join("p");
        std::fs::create_dir_all(&plugin).unwrap();
        for name in ["old", "mid", "new"] {
            std::fs::write(plugin.join(name), vec![0u8; 100]).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        prune(dir.path(), 250);
        assert!(!plugin.join("old").exists());
        assert!(plugin.join("new").exists());
    }
}
