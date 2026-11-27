use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::Result;

use crate::board::{self, Board};

const TMP_MAX_AGE: Duration = Duration::from_secs(3_600);

pub const ENV_ROOTS: [&str; 3] = ["WONTON_APP_ROOT", "WONTON_SHARED_ROOT", "WONTON_TEMP_ROOT"];

/// 三个 root（App、Shared、Temp）来自各自的 bin 启动环境；同一个目录只扫一次。
/// 只清死锁、旧临时文件和没人认领的收尾记录，活进程的东西不碰。
pub fn sweep_roots(roots: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let mut seen = Vec::new();
    let mut removed = Vec::new();
    for root in roots {
        if seen.contains(root) || !root.is_dir() {
            continue;
        }
        seen.push(root.clone());
        sweep_root(root, &mut removed)?;
    }
    Ok(removed)
}

fn sweep_root(root: &Path, removed: &mut Vec<PathBuf>) -> Result<()> {
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with(".state.toml.") && name.ends_with(".tmp") && expired(&path)? {
                remove(&path, removed);
            }
        }
    }

    let lock = root.join(".state.toml.lock");
    match board::lock_owner(root) {
        Some(owner) if owner.alive() => {}
        _ => remove(&lock, removed),
    }

    let notes = crate::verify::notes_path(root);
    let orphan = Board::open_existing(root)
        .map(|board| board.state().records.is_empty())
        .unwrap_or(true);
    if orphan {
        remove(&notes, removed);
        remove(&notes.with_extension("json.tmp"), removed);
    }
    Ok(())
}

fn expired(path: &Path) -> Result<bool> {
    let modified = std::fs::metadata(path)?.modified()?;
    Ok(SystemTime::now()
        .duration_since(modified)
        .is_ok_and(|age| age > TMP_MAX_AGE))
}

fn remove(path: &Path, removed: &mut Vec<PathBuf>) {
    if std::fs::remove_file(path).is_ok() {
        removed.push(path.to_owned());
    }
}

/// 从环境变量取三个 root；主进程按 root 兜底。
pub fn roots_from_env(fallback: &Path) -> Vec<PathBuf> {
    ENV_ROOTS
        .iter()
        .map(|key| {
            std::env::var_os(key)
                .map(PathBuf::from)
                .unwrap_or_else(|| fallback.to_owned())
        })
        .collect()
}
