//! 插件包里被 compose 和清单引用的文件：路径规则，以及从插件包目录里安全地读出它们。
//!
//! 引用的路径相对 compose 文件所在的目录，不能是绝对路径、不能含 `..`，也不能经过软链接：读文件时
//! 逐级 lstat，遇到软链接就拒绝，不让插件包借软链接把宿主上的文件带进插件的容器。

use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path};

/// 被引用的文件：规范化后的相对路径 → 内容。
pub type PackageFiles = BTreeMap<String, Vec<u8>>;

/// 插件包目录里的 compose 文件名。
pub const COMPOSE_FILE: &str = "docker-compose.yml";

/// 单个文件（包括 compose 文件本身）的大小上限。
pub const MAX_FILE_BYTES: u64 = 1 << 20;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum FileError {
    #[error("不存在")]
    Missing,
    #[error("`{0}` 是软链接，插件包里的文件不能经过软链接")]
    Symlink(String),
    #[error("`{0}` 不是目录")]
    NotDir(String),
    #[error("不是普通文件")]
    NotFile,
    #[error("超过 {} KiB", MAX_FILE_BYTES >> 10)]
    TooBig,
    #[error("读取时被替换")]
    Replaced,
    #[error("读取失败：{0}")]
    Io(String),
}

/// 规范化引用的路径：去掉 `.` 和重复的 `/`。绝对路径、`..`、空路径和变量都拒绝：路径在解析插件包时
/// 就要确定，不随安装环境变化。
pub fn normalize(raw: &str) -> Result<String, String> {
    if raw.contains('$') {
        return Err(format!("路径 `{raw}` 不能用变量"));
    }
    let mut parts = Vec::new();
    for component in Path::new(raw).components() {
        match component {
            Component::Normal(part) => parts.push(
                part.to_str()
                    .ok_or_else(|| format!("路径 `{raw}` 不是合法的 UTF-8"))?,
            ),
            Component::CurDir => {}
            Component::RootDir | Component::Prefix(_) => {
                return Err(format!("路径 `{raw}` 必须相对 compose 文件所在的目录"));
            }
            Component::ParentDir => return Err(format!("路径 `{raw}` 不能包含 ..")),
        }
    }
    if parts.is_empty() {
        return Err(format!("路径 `{raw}` 为空"));
    }
    Ok(parts.join("/"))
}

/// 从插件包目录里读出 `rel`（已规范化）：逐级 lstat，路径上有软链接、中间一级不是目录、最后一级不是
/// 普通文件或超过大小上限都拒绝。打开后再核对一次是同一个文件，防止检查之后被换成软链接。
pub fn read(dir: &Path, rel: &str) -> Result<Vec<u8>, FileError> {
    let io = |e: std::io::Error| FileError::Io(e.to_string());
    let parts: Vec<&str> = rel.split('/').collect();
    let mut path = dir.to_path_buf();
    let mut last = None;
    for (index, part) in parts.iter().enumerate() {
        path.push(part);
        let meta = fs::symlink_metadata(&path).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => FileError::Missing,
            _ => io(e),
        })?;
        let shown = || parts[..=index].join("/");
        if meta.file_type().is_symlink() {
            return Err(FileError::Symlink(shown()));
        }
        if index + 1 < parts.len() && !meta.is_dir() {
            return Err(FileError::NotDir(shown()));
        }
        last = Some(meta);
    }
    let meta = last.ok_or(FileError::Missing)?;
    if !meta.is_file() {
        return Err(FileError::NotFile);
    }
    if meta.len() > MAX_FILE_BYTES {
        return Err(FileError::TooBig);
    }
    let file = fs::File::open(&path).map_err(io)?;
    let opened = file.metadata().map_err(io)?;
    if (opened.dev(), opened.ino()) != (meta.dev(), meta.ino()) {
        return Err(FileError::Replaced);
    }
    let mut data = Vec::new();
    file.take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut data)
        .map_err(io)?;
    if data.len() as u64 > MAX_FILE_BYTES {
        return Err(FileError::TooBig);
    }
    Ok(data)
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::symlink;

    use super::*;

    #[test]
    fn references_are_normalized_or_refused() {
        for (raw, normalized) in [
            ("config.yaml", "config.yaml"),
            ("./conf//app.yaml", "conf/app.yaml"),
            ("a/./b", "a/b"),
        ] {
            assert_eq!(normalize(raw).unwrap(), normalized, "{raw}");
        }
        for bad in ["/etc/passwd", "../x", "a/../b", "", ".", "${X}/a"] {
            assert!(normalize(bad).is_err(), "{bad} 应被拒绝");
        }
    }

    #[test]
    fn files_are_read_without_following_symlinks() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("pkg");
        fs::create_dir_all(dir.join("conf")).unwrap();
        fs::write(dir.join("conf/app.yaml"), "a: 1").unwrap();
        assert_eq!(read(&dir, "conf/app.yaml").unwrap(), b"a: 1");

        // 包里的软链接指向宿主上的文件或目录：不能把它们带进插件。
        fs::write(tmp.path().join("secret"), "host secret").unwrap();
        symlink(tmp.path().join("secret"), dir.join("link.yaml")).unwrap();
        symlink(tmp.path(), dir.join("up")).unwrap();
        let cases = [
            ("link.yaml", FileError::Symlink("link.yaml".into())),
            ("up/secret", FileError::Symlink("up".into())),
            ("conf", FileError::NotFile),
            ("missing.yaml", FileError::Missing),
            ("conf/app.yaml/x", FileError::NotDir("conf/app.yaml".into())),
        ];
        for (rel, expected) in cases {
            assert_eq!(read(&dir, rel), Err(expected), "{rel}");
        }

        let big = vec![b'x'; MAX_FILE_BYTES as usize + 1];
        fs::write(dir.join("big.bin"), big).unwrap();
        assert_eq!(read(&dir, "big.bin"), Err(FileError::TooBig));
    }
}
