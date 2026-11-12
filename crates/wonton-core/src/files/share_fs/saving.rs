//! Bot 存文件（ADR-0027）：outbox 里的文件复制进暂存，判定之后放进共享文件夹。只新增，不覆盖。
//!
//! outbox 是插件写得到的目录，从应用数据目录开始逐级 `openat`（`O_NOFOLLOW`），确认期间它再怎么改，放的都是暂存里的内容。

use std::collections::HashSet;
use std::fs::{self, DirBuilder, File};
use std::io::{self, Read, Write};
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};

use rustix::fs::{AtFlags, Dir, FileType, Mode, OFlags, mkdirat, openat, unlinkat};
use rustix::io::Errno;
use sha2::{Digest, Sha256};
use wonton_protocol::FileFailure;

use super::placing::{give_to, link_to_temp, through_temp};
use super::removal::remove_entry;
use super::{
    Names, Opened, ShareError, Shares, Snapshot, UPLOAD_TEMP_PREFIX, check_public, check_segment,
    classify, from_io, open_absolute, open_regular, open_subdir, sanitize_name, stat_entry,
    sync_dir,
};
use crate::files::Failure;
use crate::plugin::hostdir::Owner;
use crate::secret::random_token;

/// 应用数据目录下给 Bot 放待存文件的目录，插件自己把它挂进容器。
pub const OUTBOX: &str = "outbox";

fn invalid(message: impl Into<String>) -> Failure {
    Failure::new(FileFailure::InvalidInput, message)
}

/// Bot 给的路径的一段。`.` 开头的名字常常是应用的配置（Syncthing 的 `.stignore`），已授权也不接受。
fn check_visible(segment: &str) -> Result<(), Failure> {
    check_segment(segment).map_err(invalid)?;
    if segment.starts_with('.') {
        return Err(invalid(format!("不接受 . 开头的名字：{segment}")));
    }
    Ok(())
}

/// 存到哪里。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveTarget {
    pub share: String,
    pub dirs: Vec<String>,
}

impl SaveTarget {
    pub fn parse(share: &str, dir: Option<&str>) -> Result<Self, Failure> {
        check_visible(share)?;
        check_public(share).map_err(Failure::from)?;
        let mut dirs = Vec::new();
        for segment in dir.into_iter().flat_map(|dir| dir.split('/')) {
            check_visible(segment)?;
            check_public(segment).map_err(Failure::from)?;
            if sanitize_name(segment) != segment {
                return Err(invalid(format!("文件夹名有不接受的字符：{segment}")));
            }
            dirs.push(segment.to_owned());
        }
        Ok(Self {
            share: share.to_owned(),
            dirs,
        })
    }

    /// `music/周杰伦`：卡片和授权都用它。
    pub fn folder(&self) -> String {
        let mut folder = self.share.clone();
        for dir in &self.dirs {
            folder.push('/');
            folder.push_str(dir);
        }
        folder
    }
}

struct Source {
    path: Vec<String>,
    name: String,
    opened: Opened,
}

/// outbox 里要存的文件，打开了，还没复制。
pub struct Outgoing {
    outbox: OwnedFd,
    sources: Vec<Source>,
}

impl Outgoing {
    pub fn open(app_data: &Path, paths: &[String]) -> Result<Self, Failure> {
        let outbox = open_outbox(app_data, "存不了文件")?;
        let mut seen = HashSet::new();
        let mut sources = Vec::with_capacity(paths.len());
        for raw in paths {
            let path: Vec<String> = raw.split('/').map(str::to_owned).collect();
            for segment in &path {
                check_visible(segment)?;
            }
            if !seen.insert(path.clone()) {
                return Err(invalid(format!("{raw} 写了两次")));
            }
            let opened = open_source(outbox.as_fd(), &path).map_err(|e| match e {
                ShareError::Gone => invalid(format!("outbox 里没有 {raw}")),
                other => other.into(),
            })?;
            let name = sanitize_name(path.last().map_or("", String::as_str));
            if name.starts_with('.') {
                return Err(invalid(format!("不接受 . 开头的名字：{raw}")));
            }
            sources.push(Source { path, name, opened });
        }
        Ok(Self { outbox, sources })
    }

    pub fn total(&self) -> u64 {
        self.sources
            .iter()
            .map(|source| source.opened.snapshot.size)
            .sum()
    }

    /// 复制进 `root` 下新建的私有目录。
    pub fn stage(self, root: &Path) -> Result<Staged, ShareError> {
        let dir = root.join(random_token());
        DirBuilder::new()
            .mode(0o700)
            .create(&dir)
            .map_err(|e| from_io(&e, "暂存目录"))?;
        let mut staged = Staged {
            outbox: self.outbox,
            dir,
            files: Vec::with_capacity(self.sources.len()),
        };
        let fd = open_absolute(&staged.dir, "暂存目录")?;
        for (index, source) in self.sources.into_iter().enumerate() {
            let stored = index.to_string();
            let sha256 = copy_hashed(&source.opened, fd.as_fd(), &stored)?;
            staged.files.push(StagedFile {
                name: source.name,
                size: source.opened.snapshot.size,
                sha256,
                stored,
                source: source.path,
                snapshot: source.opened.snapshot,
            });
        }
        Ok(staged)
    }
}

/// 打开 Bot 的 outbox：没挂载时报能分清的错（存文件和校验插件包共用，ADR-0027、ADR-0033）。
/// `why` 说清这次要做的事本来是什么，如“存不了文件”。
fn open_outbox(app_data: &Path, why: &str) -> Result<OwnedFd, Failure> {
    let app = open_absolute(app_data, "应用数据目录").map_err(|e| no_outbox(e, why))?;
    open_subdir(app.as_fd(), OUTBOX).map_err(|e| no_outbox(e, why))
}

/// Bot 的 outbox 里的一个目录，存文件和校验插件包（ADR-0033）同一套路径规则：相对路径，逐级
/// `openat`（`O_NOFOLLOW`）打开，段规则和要存的文件一样。返回打开的目录和它对应的路径，调用方要经
/// 这个路径再读时，用返回的目录核对没有被换掉。
pub fn outbox_dir(app_data: &Path, raw: &str) -> Result<(OwnedFd, PathBuf), Failure> {
    let outbox = open_outbox(app_data, "校验不了插件包")?;
    let path: Vec<String> = raw.split('/').map(str::to_owned).collect();
    for segment in &path {
        check_visible(segment)?;
    }
    let dir = walk(outbox.as_fd(), &path).map_err(|e| match e {
        ShareError::Gone => invalid(format!("outbox 里没有 {raw}")),
        other => other.into(),
    })?;
    let mut joined = app_data.join(OUTBOX);
    for segment in &path {
        joined.push(segment);
    }
    Ok((dir, joined))
}

fn no_outbox(e: ShareError, why: &str) -> Failure {
    match e {
        ShareError::Gone => Failure::new(
            FileFailure::PermissionDenied,
            format!("这个插件没有挂载 ${{WONTON_APP_DATA}}/{OUTBOX}，{why}"),
        ),
        other => other.into(),
    }
}

fn open_source(outbox: BorrowedFd<'_>, path: &[String]) -> Result<Opened, ShareError> {
    let (leaf, dirs) = path
        .split_last()
        .ok_or_else(|| ShareError::Refused("路径是空的".to_owned()))?;
    let dir = walk(outbox, dirs)?;
    open_regular(dir.as_fd(), leaf)
}

fn walk(from: BorrowedFd<'_>, dirs: &[String]) -> Result<OwnedFd, ShareError> {
    let mut current = from
        .try_clone_to_owned()
        .map_err(|e| from_io(&e, "outbox"))?;
    for dir in dirs {
        current = open_subdir(current.as_fd(), dir)?;
    }
    Ok(current)
}

fn copy_hashed(source: &Opened, dir: BorrowedFd<'_>, stored: &str) -> Result<String, ShareError> {
    let flags = OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let fd =
        openat(dir, stored, flags, Mode::from_raw_mode(0o664)).map_err(|e| classify(e, stored))?;
    let mut target = File::from(fd);
    let size = source.snapshot.size;
    let hash = (|| -> io::Result<String> {
        let mut from = (&source.file).take(size);
        let mut hasher = Sha256::new();
        let mut buffer = vec![0; 64 * 1024];
        let mut copied = 0;
        loop {
            let read = from.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
            target.write_all(&buffer[..read])?;
            copied += read as u64;
        }
        if copied != size {
            return Err(io::Error::other(format!(
                "只读到 {copied} 字节，应该有 {size} 字节"
            )));
        }
        target.sync_all()?;
        Ok(format!("{:x}", hasher.finalize()))
    })();
    hash.map_err(|e| from_io(&e, "复制进暂存目录"))
}

/// 暂存里的文件。
#[derive(Debug)]
pub struct StagedFile {
    /// 放进共享文件夹时用的名字，重名时再加序号。
    pub name: String,
    pub size: u64,
    pub sha256: String,
    stored: String,
    source: Vec<String>,
    snapshot: Snapshot,
}

/// 一次保存的暂存。不管走哪条路结束都删掉；核心崩溃时留下的由 [`reset_staging`] 清掉。
#[derive(Debug)]
pub struct Staged {
    outbox: OwnedFd,
    dir: PathBuf,
    pub files: Vec<StagedFile>,
}

impl Drop for Staged {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.dir) {
            tracing::warn!(dir = %self.dir.display(), %error, "删除暂存失败，下次启动时再清");
        }
    }
}

impl Staged {
    /// 原文件还是暂存时的样子才删，放好之后 Bot 又写的新内容留给它。删不了不算存失败。
    fn remove_original(&self, file: &StagedFile) {
        let removed = (|| -> Result<(), ShareError> {
            let (leaf, dirs) = file
                .source
                .split_last()
                .ok_or_else(|| ShareError::Refused("路径是空的".to_owned()))?;
            let dir = walk(self.outbox.as_fd(), dirs)?;
            if open_regular(dir.as_fd(), leaf)?.snapshot == file.snapshot {
                unlinkat(dir.as_fd(), leaf.as_str(), AtFlags::empty())
                    .map_err(|e| classify(e, leaf))?;
            }
            Ok(())
        })();
        if let Err(error) = removed
            && !matches!(error, ShareError::Gone)
        {
            tracing::warn!(file = %file.name, %error, "删除 outbox 里的原文件失败");
        }
    }
}

/// 启动时：清掉上次没收拾的暂存，建好目录。
pub fn reset_staging(root: &Path) -> io::Result<()> {
    match fs::remove_dir_all(root) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    DirBuilder::new().mode(0o700).create(root)
}

/// 放的结果：一个文件放不下时停下，前面放好的不撤回。
#[derive(Debug)]
pub struct Placed {
    /// `music/周杰伦/a (1).mp3`：共享文件夹开头的最后的路径。
    pub saved: Vec<String>,
    pub failed: Option<ShareError>,
}

impl Shares {
    /// 共享文件夹的名字，按名字排序。不列 `.` 开头的：回收槽这类核心自己的东西。
    pub fn names(&self) -> Result<Vec<String>, ShareError> {
        let root = open_absolute(&self.root, "共享文件夹的根目录")?;
        let mut names = Vec::new();
        for entry in Dir::read_from(&root).map_err(|e| classify(e, "共享文件夹的根目录"))?
        {
            let entry = entry.map_err(|e| classify(e, "共享文件夹的根目录"))?;
            let Ok(name) = entry.file_name().to_str() else {
                continue;
            };
            if name.starts_with('.') {
                continue;
            }
            let stat = stat_entry(root.as_fd(), name)?;
            if FileType::from_raw_mode(stat.st_mode) == FileType::Directory {
                names.push(name.to_owned());
            }
        }
        names.sort();
        Ok(names)
    }

    /// 共享文件夹还在吗。Bot 建不了新的共享文件夹。
    pub fn check_share(&self, share: &str) -> Result<(), Failure> {
        self.open_dir(share, &[]).map(drop).map_err(|e| match e {
            ShareError::Gone => invalid(format!(
                "没有共享文件夹 {share}，用 wonton__list_shares 看有哪些"
            )),
            other => other.into(),
        })
    }

    pub fn place_staged(&self, staged: &Staged, target: &SaveTarget, owner: Owner) -> Placed {
        let mut placed = Placed {
            saved: Vec::with_capacity(staged.files.len()),
            failed: None,
        };
        let opened = open_absolute(&staged.dir, "暂存目录")
            .and_then(|staging| Ok((staging, self.open_or_create(target, owner)?)));
        let (staging, dir) = match opened {
            Ok(opened) => opened,
            Err(e) => {
                placed.failed = Some(e);
                return placed;
            }
        };
        let folder = target.folder();
        for file in &staged.files {
            match place_one(staging.as_fd(), file, dir.as_fd(), owner) {
                Ok(name) => {
                    placed.saved.push(format!("{folder}/{name}"));
                    staged.remove_original(file);
                }
                Err(e) => {
                    placed.failed = Some(e);
                    break;
                }
            }
        }
        placed
    }

    /// 逐级打开，没有的子目录建出来。路径上任何一级是软链接都拒绝。
    fn open_or_create(&self, target: &SaveTarget, owner: Owner) -> Result<OwnedFd, ShareError> {
        let mut current = self.open_dir(&target.share, &[])?;
        for dir in &target.dirs {
            current = match open_subdir(current.as_fd(), dir) {
                Ok(next) => next,
                Err(ShareError::Gone) => create_subdir(current.as_fd(), dir, owner)?,
                Err(e) => return Err(e),
            };
        }
        Ok(current)
    }
}

fn create_subdir(parent: BorrowedFd<'_>, name: &str, owner: Owner) -> Result<OwnedFd, ShareError> {
    match mkdirat(parent, name, Mode::from_raw_mode(0o775)) {
        Ok(()) | Err(Errno::EXIST) => {}
        Err(e) => return Err(classify(e, name)),
    }
    let dir = File::from(open_subdir(parent, name).map_err(|e| match e {
        // 上面查过没有这个目录，名字却被占了：是个文件。
        ShareError::Gone => ShareError::Refused(format!("{name} 已经有了，不是文件夹")),
        other => other,
    })?);
    give_to(&dir, owner).map_err(|e| from_io(&e, name))?;
    sync_dir(parent, name)?;
    Ok(OwnedFd::from(dir))
}

fn place_one(
    staging: BorrowedFd<'_>,
    file: &StagedFile,
    dir: BorrowedFd<'_>,
    owner: Owner,
) -> Result<String, ShareError> {
    let source = open_regular(staging, &file.stored)?;
    let id = random_token();
    let names = Names {
        preferred: &file.name,
        owner,
        placing_inode: None,
    };
    let (name, _) = through_temp(&source, &id, dir, names, &mut |_, _| Ok(()), |temp| {
        link_to_temp(staging, &file.stored, dir, temp, &source.file)
    })?;
    let temp = format!("{UPLOAD_TEMP_PREFIX}{id}");
    if let Err(error) = remove_entry(dir, &temp) {
        tracing::warn!(name = temp, %error, "删除放文件的临时名字失败");
    }
    Ok(name)
}
