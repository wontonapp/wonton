//! 插件包里被 compose 和清单引用的文件：路径规则，以及从插件包目录里安全地读出它们。
//!
//! 引用的路径相对 compose 文件所在的目录，不能是绝对路径、不能含 `..`，也不能经过软链接：目录和文件都
//! 逐级 `openat`（`O_NOFOLLOW`）打开，遇到软链接就拒绝，不让插件包借软链接把宿主上的文件带进插件的容器。
//! 从路径打开的目录和从调用方（如 outbox 的解析）交来的目录 fd 走同一套实现，中途换不掉。

use std::collections::BTreeMap;
use std::io::Read;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::path::{Component, Path};

use rustix::fs::{CWD, FileType, Mode, OFlags, openat, statat};
use rustix::io::Errno;

/// 被引用的文件：规范化后的相对路径 → 内容。
pub type PackageFiles = BTreeMap<String, Vec<u8>>;

/// 插件包目录里的 compose 文件名。
pub const COMPOSE_FILE: &str = "docker-compose.yml";

/// 单个文件（包括 compose 文件本身）的大小上限。和 Bot 头像的上限一样，清单里的默认状态头像才装得进包（ADR-0044）。
pub const MAX_FILE_BYTES: u64 = wonton_protocol::MAX_AVATAR_BYTES;

/// 整个插件包（compose 文件和它引用的文件）的大小上限，和上传的插件包压缩包一样：一套默认状态头像（4 段视频、
/// 4 张首帧和静态头像）连同 compose 装得下。
pub const MAX_PACKAGE_BYTES: u64 = wonton_protocol::MAX_PLUGIN_ARCHIVE_BYTES;

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
    #[error("不是 UTF-8 文本")]
    NotText,
    #[error("超过 {} KiB", MAX_FILE_BYTES >> 10)]
    TooBig,
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

/// 打开插件包目录：逐级 `openat`（`O_NOFOLLOW`），任何一级是软链接或不是目录都拒绝。给 [`read_at`]
/// 和按路径的入口用。
pub fn open_dir(dir: &Path) -> Result<OwnedFd, FileError> {
    let shown = || dir.display().to_string();
    let mut parts = dir.components().peekable();
    let mut current = match parts.peek() {
        Some(Component::RootDir) | None => {
            openat(CWD, "/", open_flags(), Mode::empty()).map_err(|e| dir_error(e, shown))?
        }
        _ => openat(CWD, ".", open_flags(), Mode::empty()).map_err(|e| dir_error(e, shown))?,
    };
    for component in parts {
        let name: &std::ffi::OsStr = match component {
            Component::RootDir | Component::CurDir => continue,
            Component::Normal(part) => part,
            Component::ParentDir => "..".as_ref(),
            Component::Prefix(_) => return Err(FileError::NotDir(shown())),
        };
        current = open_component(&current, name, true, shown)?;
    }
    Ok(current)
}

/// 中间目录和叶子文件都不带 `DIRECTORY`：带着它时软链接报的是 ENOTDIR，分不清是不是软链接。
/// `NONBLOCK`：换成命名管道时不能卡住。打开后用 [`FileError`] 核对类型。
fn open_flags() -> OFlags {
    OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK
}

/// 打开路径上的一级并核对类型：`want_dir` 时必须是目录，否则按 `NotDir` 拒绝；软链接按 `Symlink`。
fn open_component(
    dir: &OwnedFd,
    name: &std::ffi::OsStr,
    want_dir: bool,
    shown: impl Fn() -> String,
) -> Result<OwnedFd, FileError> {
    let fd = match openat(dir, name, open_flags(), Mode::empty()) {
        Ok(fd) => fd,
        Err(Errno::LOOP) => return Err(FileError::Symlink(shown())),
        Err(e) => return Err(dir_error(e, shown)),
    };
    let meta =
        statat(&fd, "", rustix::fs::AtFlags::EMPTY_PATH).map_err(|e| dir_error(e, &shown))?;
    let is_dir = FileType::from_raw_mode(meta.st_mode) == FileType::Directory;
    if want_dir && !is_dir {
        return Err(FileError::NotDir(shown()));
    }
    Ok(fd)
}

fn dir_error(e: Errno, shown: impl Fn() -> String) -> FileError {
    match e {
        Errno::NOENT => FileError::Missing,
        _ => FileError::Io(format!("{}：{e}", shown())),
    }
}

/// 从已打开的插件包目录读出 `rel`（已规范化）：逐级 `openat`（`O_NOFOLLOW`），路径上有软链接、中间一级
/// 不是目录、最后一级不是普通文件或超过大小上限都拒绝。没有检查后换文件的窗口：fd 一路钉住。
pub fn read_at(dir: BorrowedFd<'_>, rel: &str) -> Result<Vec<u8>, FileError> {
    let parts: Vec<&str> = rel.split('/').collect();
    let mut current = dir.try_clone_to_owned().map_err(|e| io_error(&e, rel))?;
    for (index, part) in parts.iter().enumerate() {
        let shown = || parts[..=index].join("/");
        // 中间一级必须是目录；叶子是什么类型打开后核对（普通文件才算数）。
        current = open_component(&current, (*part).as_ref(), index + 1 < parts.len(), shown)?;
    }
    let meta = statat(&current, "", rustix::fs::AtFlags::EMPTY_PATH)
        .map_err(|e| dir_error(e, || rel.to_owned()))?;
    if FileType::from_raw_mode(meta.st_mode) != FileType::RegularFile {
        return Err(FileError::NotFile);
    }
    if meta.st_size < 0 || (meta.st_size as u64) > MAX_FILE_BYTES {
        return Err(FileError::TooBig);
    }
    let file = std::fs::File::from(current);
    let mut data = Vec::new();
    file.take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut data)
        .map_err(|e| io_error(&e, rel))?;
    if data.len() as u64 > MAX_FILE_BYTES {
        return Err(FileError::TooBig);
    }
    Ok(data)
}

fn io_error(e: &std::io::Error, what: &str) -> FileError {
    FileError::Io(format!("{what}：{e}"))
}

/// 从插件包目录的路径读出 `rel`：先按同一套规则打开目录，再 [`read_at`]。
pub fn read(dir: &Path, rel: &str) -> Result<Vec<u8>, FileError> {
    read_at(open_dir(dir)?.as_fd(), rel)
}

/// 把插件包打成 tar 归档，[`super::package_archive`] 读得回来：compose 文件在前，被引用的文件按路径排序；修改时间写 0，
/// 同一个插件包每次打出来都一样。
pub fn write_archive(compose: &[u8], files: &PackageFiles) -> Vec<u8> {
    let mut builder = tar::Builder::new(Vec::new());
    let entries = std::iter::once((COMPOSE_FILE, compose)).chain(
        files
            .iter()
            .map(|(path, data)| (path.as_str(), data.as_slice())),
    );
    for (path, data) in entries {
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Regular);
        header.set_size(data.len() as u64);
        header.set_mode(0o644);
        header.set_mtime(0);
        builder
            .append_data(&mut header, path, data)
            .expect("路径都经过 normalize，写进内存不会失败");
    }
    builder.into_inner().expect("写进内存不会失败")
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::symlink;

    use super::*;

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
