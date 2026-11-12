//! 静态网页包里 `static_web.root` 下的文件（ADR-0080 第 3 节）：从插件包目录、压缩包或逐个取来的文件里收齐，
//! 按目录规则核对。内容只在装、升级时带着；装好后安装记录只留每个文件的 sha256（算插件包摘要用），
//! 文件本身只在托管目录里（见 [`crate::plugin::hosted`]）。

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};

use rustix::fs::{AtFlags, Dir, FileType, Mode, OFlags, openat, statat};
use rustix::io::Errno;

use super::super::languages::Languages;
use super::super::manifest_static::StaticWebSpec;
use super::super::package_files::sha256_hex;
use crate::l10n::text;

/// `root` 下最多几项（文件和目录一起数）。
pub const MAX_ENTRIES: usize = 1024;
/// `root` 下的文件合计多大。
pub const MAX_TOTAL_BYTES: u64 = 64 << 20;
/// 单个文件多大：前端打包出的 JS 和 wasm 常超过插件包引用文件的 2 MiB 上限。
pub const MAX_FILE_BYTES: u64 = 16 << 20;
/// 每个静态网页必须有的首页。
pub const INDEX: &str = "index.html";
/// 网关占用的路径前缀（数据存储），包里不能有这个目录。
pub const RESERVED: &str = "_wonton";

/// 相对 `static_web.root` 的路径 → 内容。
pub type StaticFiles = BTreeMap<String, Vec<u8>>;
/// 相对 `static_web.root` 的路径 → sha256 的十六进制。
pub type StaticHashes = BTreeMap<String, String>;

/// 目录不合规则的原因；文字按请求的语言在 [`Problem::message`] 里写。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    TooMany,
    TooBig(u64),
    FileTooBig(String),
    Hidden(String),
    Reserved,
    NoIndex,
    NoFallback(String),
    Symlink(String),
    NotFile(String),
    Missing,
    Io(String, String),
}

impl Problem {
    pub fn message(&self, root: &str, langs: &Languages) -> String {
        let mib = |bytes: u64| (bytes >> 20).to_string();
        let problem = match self {
            Self::TooMany => text(
                langs,
                "plugin-install-static-too-many",
                &[("max", MAX_ENTRIES.to_string().into())],
            ),
            Self::TooBig(total) => text(
                langs,
                "plugin-install-static-too-big",
                &[
                    ("mib", mib(*total).into()),
                    ("max", mib(MAX_TOTAL_BYTES).into()),
                ],
            ),
            Self::FileTooBig(path) => text(
                langs,
                "plugin-install-static-file-too-big",
                &[
                    ("path", path.as_str().into()),
                    ("max", mib(MAX_FILE_BYTES).into()),
                ],
            ),
            Self::Hidden(path) => text(
                langs,
                "plugin-install-static-hidden",
                &[("path", path.as_str().into())],
            ),
            Self::Reserved => text(langs, "plugin-install-static-reserved", &[]),
            Self::NoIndex => text(langs, "plugin-install-static-no-index", &[]),
            Self::NoFallback(path) => text(
                langs,
                "plugin-install-static-no-fallback",
                &[("path", path.as_str().into())],
            ),
            Self::Symlink(path) => text(
                langs,
                "plugin-install-static-symlink",
                &[("path", path.as_str().into())],
            ),
            Self::NotFile(path) => text(
                langs,
                "plugin-install-static-not-file",
                &[("path", path.as_str().into())],
            ),
            Self::Missing => text(langs, "plugin-install-static-missing", &[]),
            Self::Io(path, error) => text(
                langs,
                "plugin-install-static-read-failed",
                &[
                    ("path", path.as_str().into()),
                    ("error", error.as_str().into()),
                ],
            ),
        };
        text(
            langs,
            "plugin-install-static-dir",
            &[("root", root.into()), ("problem", problem.into())],
        )
    }
}

/// 一条相对 `root` 的路径能不能放进托管目录：每段非空、不以 `.` 开头，第一段不是 `_wonton`。
pub fn check_path(rel: &str) -> Result<(), Problem> {
    let parts: Vec<&str> = rel.split('/').collect();
    if parts
        .iter()
        .any(|part| part.is_empty() || part.starts_with('.') || part.contains(['\\', '\0']))
    {
        return Err(Problem::Hidden(rel.to_owned()));
    }
    if parts.len() > 1 && parts[0] == RESERVED {
        return Err(Problem::Reserved);
    }
    Ok(())
}

/// 整个目录的规则：项数、大小、首页和 `fallback`。路径逐条按 [`check_path`]。
pub fn check(files: &StaticFiles, spec: &StaticWebSpec) -> Result<(), Problem> {
    let mut dirs = BTreeSet::new();
    let mut total = 0u64;
    for (path, data) in files {
        check_path(path)?;
        if data.len() as u64 > MAX_FILE_BYTES {
            return Err(Problem::FileTooBig(path.clone()));
        }
        total += data.len() as u64;
        let mut parent = path.as_str();
        while let Some((dir, _)) = parent.rsplit_once('/') {
            dirs.insert(dir);
            parent = dir;
        }
    }
    if files.len() + dirs.len() > MAX_ENTRIES {
        return Err(Problem::TooMany);
    }
    if total > MAX_TOTAL_BYTES {
        return Err(Problem::TooBig(total));
    }
    if !files.contains_key(INDEX) {
        return Err(Problem::NoIndex);
    }
    if let Some(fallback) = &spec.fallback
        && !files.contains_key(fallback)
    {
        return Err(Problem::NoFallback(fallback.clone()));
    }
    Ok(())
}

pub fn hashes(files: &StaticFiles) -> StaticHashes {
    files
        .iter()
        .map(|(path, data)| (path.clone(), sha256_hex(data)))
        .collect()
}

/// 包里 `root` 目录下的全部文件。逐级 `openat`（`O_NOFOLLOW`）：软链接、设备、管道都拒绝，读的同时数项数和大小，
/// 不先把超限的目录整个读进内存。
pub fn read_dir(package: BorrowedFd<'_>, root: &str) -> Result<StaticFiles, Problem> {
    let mut current = package
        .try_clone_to_owned()
        .map_err(|e| Problem::Io(root.to_owned(), e.to_string()))?;
    for part in root.split('/') {
        current = open_dir(&current, part, root)?;
    }
    let mut walk = Walk::default();
    walk.dir(&current, "")?;
    Ok(walk.files)
}

#[derive(Default)]
struct Walk {
    files: StaticFiles,
    entries: usize,
    total: u64,
}

impl Walk {
    fn dir(&mut self, dir: &OwnedFd, prefix: &str) -> Result<(), Problem> {
        let io = |e: Errno| Problem::Io(prefix.to_owned(), e.to_string());
        let mut names = Vec::new();
        for entry in Dir::read_from(dir).map_err(io)? {
            let name = entry
                .map_err(io)?
                .file_name()
                .to_string_lossy()
                .into_owned();
            if name != "." && name != ".." {
                names.push(name);
            }
        }
        names.sort();
        for name in names {
            let rel = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}/{name}")
            };
            check_path(&rel)?;
            self.entries += 1;
            if self.entries > MAX_ENTRIES {
                return Err(Problem::TooMany);
            }
            let meta = statat(dir, name.as_str(), AtFlags::SYMLINK_NOFOLLOW)
                .map_err(|e| Problem::Io(rel.clone(), e.to_string()))?;
            match FileType::from_raw_mode(meta.st_mode) {
                FileType::Directory => {
                    let child = open_dir(dir, &name, &rel)?;
                    self.dir(&child, &rel)?;
                }
                FileType::RegularFile => self.file(dir, &name, rel)?,
                FileType::Symlink => return Err(Problem::Symlink(rel)),
                _ => return Err(Problem::NotFile(rel)),
            }
        }
        Ok(())
    }

    fn file(&mut self, dir: &OwnedFd, name: &str, rel: String) -> Result<(), Problem> {
        let fd = match openat(dir, name, flags(), Mode::empty()) {
            Ok(fd) => fd,
            Err(Errno::LOOP) => return Err(Problem::Symlink(rel)),
            Err(e) => return Err(Problem::Io(rel, e.to_string())),
        };
        let meta = statat(&fd, "", AtFlags::EMPTY_PATH)
            .map_err(|e| Problem::Io(rel.clone(), e.to_string()))?;
        if FileType::from_raw_mode(meta.st_mode) != FileType::RegularFile {
            return Err(Problem::NotFile(rel));
        }
        let mut data = Vec::new();
        std::fs::File::from(fd)
            .take(MAX_FILE_BYTES + 1)
            .read_to_end(&mut data)
            .map_err(|e| Problem::Io(rel.clone(), e.to_string()))?;
        if data.len() as u64 > MAX_FILE_BYTES {
            return Err(Problem::FileTooBig(rel));
        }
        self.total += data.len() as u64;
        if self.total > MAX_TOTAL_BYTES {
            return Err(Problem::TooBig(self.total));
        }
        self.files.insert(rel, data);
        Ok(())
    }
}

/// `NONBLOCK`：换成命名管道时不能卡住；不带 `DIRECTORY`，软链接才报 `ELOOP` 而不是 `ENOTDIR`。
fn flags() -> OFlags {
    OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK
}

fn open_dir(parent: &OwnedFd, name: &str, shown: &str) -> Result<OwnedFd, Problem> {
    let fd = match openat(parent, name, flags(), Mode::empty()) {
        Ok(fd) => fd,
        Err(Errno::LOOP) => return Err(Problem::Symlink(shown.to_owned())),
        Err(Errno::NOENT) => return Err(Problem::Missing),
        Err(e) => return Err(Problem::Io(shown.to_owned(), e.to_string())),
    };
    let meta = statat(fd.as_fd(), "", AtFlags::EMPTY_PATH)
        .map_err(|e| Problem::Io(shown.to_owned(), e.to_string()))?;
    if FileType::from_raw_mode(meta.st_mode) != FileType::Directory {
        return Err(Problem::NotFile(shown.to_owned()));
    }
    Ok(fd)
}
