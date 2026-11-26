//! 公网访问总开关（ADR-0090 第 2 节）：默认关，只有主人能开，关着时公网入口全部拒绝。
//!
//! 开关存在核心数据目录的 `core/clients/public-access.json`（0600）。读不出来按关处理：
//! 关是安全的一侧，App 显示总开关关着，主人再开一次就恢复。

use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tracing::warn;

const FILE: &str = "core/clients/public-access.json";

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
struct Stored {
    #[serde(default)]
    enabled: bool,
}

fn path(data_dir: &Path) -> PathBuf {
    data_dir.join(FILE)
}

/// 读总开关；文件不在或坏了都按关处理。
pub fn load(data_dir: &Path) -> bool {
    let path = path(data_dir);
    match std::fs::read(&path) {
        Ok(bytes) => match serde_json::from_slice::<Stored>(&bytes) {
            Ok(stored) => stored.enabled,
            Err(error) => {
                warn!(path = %path.display(), %error, "公网访问总开关读不了，按关处理");
                false
            }
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => {
            warn!(path = %path.display(), %error, "公网访问总开关读不了，按关处理");
            false
        }
    }
}

/// 原子地写总开关：临时文件写好、同步、改名，权限 0600。
pub fn save(data_dir: &Path, enabled: bool) -> std::io::Result<()> {
    let path = path(data_dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp)?;
    let bytes = serde_json::to_vec(&Stored { enabled }).map_err(std::io::Error::other)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    #[test]
    fn missing_and_broken_files_read_as_off() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!load(dir.path()));
        std::fs::create_dir_all(dir.path().join("core/clients")).unwrap();
        std::fs::write(path(dir.path()), b"not json").unwrap();
        assert!(!load(dir.path()));
    }

    #[test]
    fn round_trip_and_mode() {
        let dir = tempfile::tempdir().unwrap();
        save(dir.path(), true).unwrap();
        assert!(load(dir.path()));
        save(dir.path(), false).unwrap();
        assert!(!load(dir.path()));
        assert_eq!(
            std::fs::metadata(path(dir.path()))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}
