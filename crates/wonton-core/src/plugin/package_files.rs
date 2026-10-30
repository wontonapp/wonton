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

/// 整个插件包（compose 文件和它引用的文件）的大小上限。
pub const MAX_PACKAGE_BYTES: u64 = 2 << 20;

/// 插件包摘要（插件规范的“插件包摘要”）：每个文件一行 `<sha256>  <路径>`，按路径的字节序排列，即 `sha256sum`
/// 的输出；这段文字的 sha256 写成 `sha256:<十六进制>`。只取决于文件的内容，和文件从哪里取、怎么打包无关。
pub fn digest(compose: &[u8], files: &PackageFiles) -> String {
    let mut hashes: BTreeMap<&str, String> = files
        .iter()
        .map(|(path, data)| (path.as_str(), sha256_hex(data)))
        .collect();
    hashes.insert(COMPOSE_FILE, sha256_hex(compose));
    let listing: String = hashes
        .iter()
        .map(|(path, hash)| format!("{hash}  {path}\n"))
        .collect();
    format!("sha256:{}", sha256_hex(listing.as_bytes()))
}

pub fn sha256_hex(data: &[u8]) -> String {
    let hash = aws_lc_rs::digest::digest(&aws_lc_rs::digest::SHA256, data);
    hash.as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

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

/// 归档里最多这么多项：插件包只有 compose 文件和它引用的少数几个文件。
const MAX_ARCHIVE_ENTRIES: usize = 256;

/// 读插件包目录打成的 tar 归档（如 `tar -C <目录> -cf - .`），返回其中的普通文件。目录跳过；软链接、硬链接、
/// 设备等一律拒绝，路径规则同 [`normalize`]，单个文件的上限同 [`read`]。
pub fn read_archive(archive: &[u8]) -> Result<PackageFiles, String> {
    let mut files = PackageFiles::new();
    let mut archive = tar::Archive::new(archive);
    let entries = archive
        .entries()
        .map_err(|e| format!("不是 tar 归档：{e}"))?;
    for (index, entry) in entries.enumerate() {
        if index >= MAX_ARCHIVE_ENTRIES {
            return Err(format!("归档里超过 {MAX_ARCHIVE_ENTRIES} 项"));
        }
        let entry = entry.map_err(|e| format!("读归档失败：{e}"))?;
        let raw = entry
            .path()
            .map_err(|e| format!("归档里的路径不合法：{e}"))?
            .to_string_lossy()
            .into_owned();
        match entry.header().entry_type() {
            tar::EntryType::Directory => continue,
            tar::EntryType::Regular | tar::EntryType::Continuous => {}
            other => {
                return Err(format!(
                    "`{raw}` 不是普通文件（{other:?}）：插件包里不能有软链接、硬链接和设备"
                ));
            }
        }
        let path = normalize(&raw)?;
        let mut data = Vec::new();
        entry
            .take(MAX_FILE_BYTES + 1)
            .read_to_end(&mut data)
            .map_err(|e| format!("读归档里的 `{path}` 失败：{e}"))?;
        if data.len() as u64 > MAX_FILE_BYTES {
            return Err(format!("`{path}` {}", FileError::TooBig));
        }
        if files.insert(path.clone(), data).is_some() {
            return Err(format!("归档里有两个 `{path}`"));
        }
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::symlink;

    use super::*;

    /// 手写的归档：路径直接写进头部，`tar::Builder` 拒绝写的 `..` 也能造出来。
    fn archive(entries: &[(&str, tar::EntryType, &[u8])]) -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        for (path, kind, data) in entries {
            let mut header = tar::Header::new_gnu();
            header.as_old_mut().name[..path.len()].copy_from_slice(path.as_bytes());
            header.set_entry_type(*kind);
            header.set_size(data.len() as u64);
            header.set_mode(0o644);
            if *kind == tar::EntryType::Symlink {
                header.set_link_name("/etc/passwd").unwrap();
            }
            header.set_cksum();
            builder.append(&header, *data).unwrap();
        }
        builder.into_inner().unwrap()
    }

    #[test]
    fn archives_yield_regular_files_and_refuse_links() {
        let regular = tar::EntryType::Regular;
        let ok = archive(&[
            ("./", tar::EntryType::Directory, b""),
            ("./docker-compose.yml", regular, b"services: {}"),
            ("./conf/app.yaml", regular, b"a: 1"),
        ]);
        let files = read_archive(&ok).unwrap();
        let names: Vec<&str> = files.keys().map(String::as_str).collect();
        assert_eq!(names, ["conf/app.yaml", "docker-compose.yml"]);
        let link = archive(&[("config.yaml", tar::EntryType::Symlink, b"")]);
        assert!(read_archive(&link).unwrap_err().contains("软链接"));
        let escape = archive(&[("../x", regular, b"x")]);
        assert!(read_archive(&escape).is_err());
        let big = vec![b'x'; MAX_FILE_BYTES as usize + 1];
        assert!(read_archive(&archive(&[("big.bin", regular, &big)])).is_err());
        assert!(read_archive(b"not a tar archive at all").is_err());
    }

    /// 摘要和 `sha256sum docker-compose.yml icon.svg | sha256sum` 一致（在 macOS 上用 `shasum -a 256` 算的）。
    #[test]
    fn digest_is_the_hash_of_the_sha256sum_listing() {
        let files = PackageFiles::from([("icon.svg".to_owned(), b"b\n".to_vec())]);
        assert_eq!(
            digest(b"a\n", &files),
            "sha256:699204d977f0cad660a051467a6b3095cb2e1e0ce7ba4eeb2e628d428b05df23"
        );
        assert_ne!(digest(b"a\n", &PackageFiles::new()), digest(b"a\n", &files));
    }

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
