//! 主人配置文件 `relays.json` 的读写：JSON 里带令牌，文件权限 0600，先写临时文件再改名。

use std::io::Write as _;
use std::path::Path;

use super::{LoadError, OwnerFile, OwnerLayer};

/// 文件权限不对时给设置页的提示：文件里有令牌，别人不该读得到。
///
/// 非 unix 没有 mode，交给平台默认；unix 上只检查 group/other 位。
pub(super) fn permission_warning(path: &Path) -> Option<String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(path).ok()?.permissions().mode() & 0o777;
        if mode & 0o077 != 0 {
            return Some(format!(
                "{} 的权限是 {mode:03o}，别的用户可能读到令牌；建议 chmod 600",
                path.display()
            ));
        }
    }
    let _ = path;
    None
}

/// 读文件。格式不对时把路径和原因一起报出来，不静默当没配。
pub(super) fn parse(path: &Path, text: &str) -> Result<OwnerLayer, LoadError> {
    let file: OwnerFile = serde_json::from_str(text).map_err(|reason| LoadError::Parse {
        path: path.display().to_string(),
        reason: reason.to_string(),
    })?;
    file.to_layer().map_err(|reason| LoadError::Parse {
        path: path.display().to_string(),
        reason,
    })
}

/// 原子写：同目录的临时文件写完、同步、再改名。文件里有令牌，权限只给本用户。
pub(super) fn write(path: &Path, file: &OwnerFile) -> std::io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    let text = serde_json::to_string_pretty(file)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(text.as_bytes())?;
    temporary.write_all(b"\n")?;
    temporary.as_file().sync_all()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        temporary
            .as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    temporary
        .persist(path)
        .map_err(|error| error.error)
        .map(|_| ())
}
