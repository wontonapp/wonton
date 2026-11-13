//! 共享文件夹里的文件：核心按名字打开，不跟随软链接。
//!
//! 有写权限的插件能把子目录换成软链接（ADR-0011），所以从共享文件夹的根开始逐级 `openat`（`O_NOFOLLOW`），后面每一级都
//! 相对上一级已经打开的目录。不用 `openat2`：老内核（如群晖）没有它。

use std::fs::File;
use std::io;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use percent_encoding::percent_decode_str;
use rustix::fs::{AtFlags, CWD, FileType, Mode, OFlags, Stat, openat, statat};
use rustix::io::Errno;
use serde::{Deserialize, Serialize};
use unicode_properties::{GeneralCategory, UnicodeGeneralCategory};

use crate::plugin::languages::Languages;
use crate::plugin::store;
pub use placing::{Abandoned, BeforeLink, Names};
use removal::{prepare_trash, recover_trash};

mod placing;
mod removal;
pub mod saving;

/// 工具结果里共享文件夹引用的前缀（插件规范的“工具结果里的文件”）。
const SHARE_SCHEME: &str = "wonton-share:";
/// Upload subdirectory inside each person's private home.
pub const TRANSFERS: &str = "transfers";
/// 文件名的上限：常见文件系统都是 255 字节，留出重名序号的位置。
const MAX_NAME_BYTES: usize = 240;
/// 重名时最多试这么多个序号。
const MAX_NAME_TRIES: u32 = 10_000;
/// 核心在 `transfers` 里的私有名字都以它开头，设备和 Bot 给的名字占不了（[`sanitize_name`]）。
const PRIVATE_PREFIX: &str = ".wonton-";
/// 放进 `transfers` 之前的临时名字：同一个卷上是数据的硬链接，不在同一个卷上是复制出来的文件。长度固定；上传编号是 128 位
/// 随机数，这个名字上的文件只可能是核心自己建的。
pub(super) const UPLOAD_TEMP_PREFIX: &str = ".wonton-upload-";

/// 只读打开路径上的一级。`O_NONBLOCK`：换成命名管道时不能卡住；`O_NOCTTY`：换成终端时不能接管它。
fn walk_flags() -> OFlags {
    OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK | OFlags::NOCTTY
}

/// 打不开的原因。
#[derive(Debug, thiserror::Error)]
pub enum ShareError {
    /// 文件或它的上级目录不在了。
    #[error("文件已被移动或删除")]
    Gone,
    /// 路径不合法：有软链接，或者不是普通文件、目录。
    #[error("{0}")]
    Refused(String),
    #[error("{0}")]
    NoSpace(String),
    #[error("{0}")]
    Io(String),
}

fn classify(e: Errno, what: &str) -> ShareError {
    match e {
        // 上级目录被换成了文件：和被移动是一回事。
        Errno::NOENT | Errno::NOTDIR => ShareError::Gone,
        Errno::LOOP => ShareError::Refused(format!("{what} 是软链接，核心不跟随")),
        // 共享文件夹在 NAS 上时配额满了也是放不下。
        Errno::NOSPC | Errno::DQUOT => ShareError::NoSpace(format!("{what}：磁盘空间不够")),
        other => ShareError::Io(format!("{what}：{other}")),
    }
}

pub(super) fn from_io(e: &io::Error, what: &str) -> ShareError {
    match e.raw_os_error() {
        Some(raw) => classify(Errno::from_raw_os_error(raw), what),
        None => ShareError::Io(format!("{what}：{e}")),
    }
}

/// 一个共享文件夹里的相对路径。分成段保存：解码后的段里不会有 `/`。
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ShareRef {
    pub share: String,
    pub path: Vec<String>,
}

impl ShareRef {
    /// 解析 `wonton-share:<共享文件夹>/<相对路径>`，路径按 URI 规则做过百分号编码。
    pub fn parse(uri: &str) -> Result<Self, String> {
        let rest = uri
            .strip_prefix(SHARE_SCHEME)
            .ok_or_else(|| format!("不是 {SHARE_SCHEME} 引用"))?;
        let mut segments = rest
            .split('/')
            .map(|raw| {
                let decoded = percent_decode_str(raw)
                    .decode_utf8()
                    .map_err(|_| "路径不是合法的 UTF-8".to_owned())?;
                check_segment(&decoded)?;
                Ok(decoded.into_owned())
            })
            .collect::<Result<Vec<String>, String>>()?
            .into_iter();
        let share = segments.next().ok_or("缺少共享文件夹的名字")?;
        let path: Vec<String> = segments.collect();
        if path.is_empty() {
            return Err("缺少共享文件夹里的路径".to_owned());
        }
        Ok(Self { share, path })
    }

    /// 工具结果里的 `uri` 是不是共享文件夹的引用。
    pub fn is_reference(uri: &str) -> bool {
        uri.starts_with(SHARE_SCHEME)
    }

    /// 路径的最后一段，`name` 没写时当文件名。
    pub fn file_name(&self) -> &str {
        self.path.last().map_or("", String::as_str)
    }
}

/// 路径的一段：不能是空的、`.`、`..`，不能含 `/`、NUL。
fn check_segment(segment: &str) -> Result<(), String> {
    match segment {
        "" => Err("路径不能有空段，也不能是绝对路径".to_owned()),
        "." | ".." => Err("路径不能含 . 或 ..".to_owned()),
        s if s.contains(['/', '\0']) => Err("路径的一段里不能有 / 或 NUL".to_owned()),
        _ => Ok(()),
    }
}

/// 核心的私有名字。不分大小写：大小写不敏感的卷上 `.WONTON-TRASH` 就是回收槽。
fn is_private(name: &str) -> bool {
    name.get(..PRIVATE_PREFIX.len())
        .is_some_and(|start| start.eq_ignore_ascii_case(PRIVATE_PREFIX))
}

/// 从外面来的路径的一段：核心可能以 root 打开，回收槽的 0700 和没放好的上传不能经它绕过去。核心用私有名字时不走这里。
fn check_public(segment: &str) -> Result<(), ShareError> {
    check_segment(segment).map_err(ShareError::Refused)?;
    if is_private(segment) {
        return Err(ShareError::Refused(format!("{segment} 是核心的私有名字")));
    }
    Ok(())
}

/// 文件当时的样子：`DELETE` 核对它，Bot 的工具用它判断文件登记后改过没有。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Snapshot {
    pub inode: u64,
    pub size: u64,
    pub modified_ns: i64,
}

impl Snapshot {
    fn of(file: &File) -> io::Result<(Self, std::fs::Metadata)> {
        let meta = file.metadata()?;
        let snapshot = Self {
            inode: meta.ino(),
            size: meta.size(),
            modified_ns: meta
                .mtime()
                .saturating_mul(1_000_000_000)
                .saturating_add(meta.mtime_nsec()),
        };
        Ok((snapshot, meta))
    }

    /// 强 `ETag`：内容变了，大小或修改时间（纳秒）就变；同一路径放了新文件，inode 也变。
    pub fn etag(&self) -> String {
        format!(
            "\"{:x}-{:x}-{:x}\"",
            self.inode, self.size, self.modified_ns
        )
    }
}

/// 打开的文件和它当时的样子。
#[derive(Debug)]
pub struct Opened {
    pub file: File,
    pub snapshot: Snapshot,
}

/// [`Shares::delete_if_unchanged`] 的结果。
#[derive(Debug, PartialEq, Eq)]
pub enum Deleted {
    /// `durable` 是假：文件删了，目录没能确认落盘，断电后可能回来。回来了也还是普通文件，主人再删一次。
    Removed { durable: bool },
    /// 本来就不在了。
    AlreadyGone,
    /// 这个路径已经是别的文件，没有动它。
    Changed,
}

/// 共享文件夹的根目录（`<数据目录>/shares`）。
#[derive(Debug, Clone)]
pub struct Shares {
    root: PathBuf,
    /// 删除排队用同一组回收槽（[`removal`]）。
    removing: Arc<Mutex<()>>,
}

impl Shares {
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            removing: Arc::default(),
        }
    }

    pub(super) fn at_root(&self, root: PathBuf) -> Self {
        Self {
            root,
            removing: self.removing.clone(),
        }
    }

    /// 逐级打开目录。路径上任何一级是软链接都拒绝，不是目录就算文件没了。
    fn open_dir(&self, share: &str, dirs: &[String]) -> Result<OwnedFd, ShareError> {
        check_public(share)?;
        // 根目录是核心建的，插件碰不到它。
        let root = open_absolute(&self.root, "共享文件夹的根目录")?;
        let mut current = open_subdir(root.as_fd(), share)?;
        for dir in dirs {
            check_public(dir)?;
            current = open_subdir(current.as_fd(), dir)?;
        }
        Ok(current)
    }

    /// 只读打开共享文件夹里的普通文件。
    pub fn open(&self, reference: &ShareRef) -> Result<Opened, ShareError> {
        let (leaf, dirs) = split_leaf(reference)?;
        let dir = self.open_dir(&reference.share, dirs)?;
        open_regular(dir.as_fd(), leaf)
    }

    /// `transfers` 里的文件还是 `expected` 就删掉，否则不动。全程只用打开的目录，路径中途被换也删不到别处。核心只删设备上传的
    /// 文件，别的共享文件夹没有回收槽。
    pub fn delete_if_unchanged(
        &self,
        reference: &ShareRef,
        expected: &Snapshot,
        langs: &Languages,
    ) -> Result<Deleted, ShareError> {
        if reference.share != TRANSFERS {
            return Err(ShareError::Refused(crate::l10n::text(
                langs,
                "files-delete-transfers-only",
                &[("folder", TRANSFERS.into())],
            )));
        }
        let (leaf, dirs) = split_leaf(reference)?;
        let dir = match self.open_dir(&reference.share, dirs) {
            Ok(dir) => dir,
            Err(ShareError::Gone) => return Ok(Deleted::AlreadyGone),
            Err(e) => return Err(e),
        };
        let opened = match open_regular(dir.as_fd(), leaf) {
            Ok(opened) => opened,
            Err(ShareError::Gone) => return Ok(Deleted::AlreadyGone),
            // 换成了软链接或者目录：已经不是上传的那个文件。
            Err(ShareError::Refused(_)) => return Ok(Deleted::Changed),
            Err(e) => return Err(e),
        };
        if opened.snapshot != *expected {
            return Ok(Deleted::Changed);
        }
        let transfers = self.open_dir(TRANSFERS, &[])?;
        let _removing = self.removing.lock().expect("删除锁中毒");
        removal::remove_if_same(transfers.as_fd(), dir.as_fd(), leaf, expected, langs)
    }

    /// 启动时：建好回收槽，建不了要让启动失败：磁盘满了再建就晚了，主人会删不了文件。
    pub fn prepare_removal(&self) -> Result<(), ShareError> {
        let _removing = self.removing.lock().expect("删除锁中毒");
        let dir = self.open_dir(TRANSFERS, &[])?;
        // 启动时没有请求头：回收槽的核对错误按英文说，只进启动日志。
        prepare_trash(dir.as_fd(), &Languages::from_header(None))
    }

    /// 启动时找回删除到一半的文件，见 [`recover_trash`]。只在启动时找：运行中槽里可能正有删除在进行。
    pub fn recover_deleting(&self) -> Result<(), ShareError> {
        let dir = self.open_dir(TRANSFERS, &[])?;
        recover_trash(dir.as_fd(), &Languages::from_header(None))
    }
}

/// 打开核心自己的目录。这一级可以跟随软链接（数据目录可能在软链接后面）。
fn open_absolute(path: &Path, what: &str) -> Result<OwnedFd, ShareError> {
    let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC;
    openat(CWD, path, flags, Mode::empty()).map_err(|e| classify(e, what))
}

fn split_leaf(reference: &ShareRef) -> Result<(&str, &[String]), ShareError> {
    for segment in &reference.path {
        check_public(segment)?;
    }
    let (leaf, dirs) = reference
        .path
        .split_last()
        .ok_or_else(|| ShareError::Refused("缺少共享文件夹里的路径".to_owned()))?;
    Ok((leaf, dirs))
}

/// 这个名字上现在是什么。不打开：打开 socket、设备本身就有副作用。
fn stat_entry(dir: BorrowedFd<'_>, name: &str) -> Result<Stat, ShareError> {
    statat(dir, name, AtFlags::SYMLINK_NOFOLLOW).map_err(|e| classify(e, name))
}

#[allow(clippy::unnecessary_cast)] // `st_dev`、`st_ino` 的类型各平台不同。
fn is_same_file(stat: &Stat, meta: &std::fs::Metadata) -> bool {
    stat.st_ino as u64 == meta.ino() && stat.st_dev as u64 == meta.dev()
}

/// 查过类型之后才打开，打开后再比对：这之间名字可能被换掉。
fn open_checked(dir: BorrowedFd<'_>, name: &str, stat: &Stat) -> Result<File, ShareError> {
    let fd = openat(dir, name, walk_flags(), Mode::empty()).map_err(|e| classify(e, name))?;
    let file = File::from(fd);
    let meta = file.metadata().map_err(|e| from_io(&e, name))?;
    if !is_same_file(stat, &meta) {
        return Err(ShareError::Refused(format!("{name} 刚被换掉了")));
    }
    Ok(file)
}

fn open_subdir(parent: BorrowedFd<'_>, name: &str) -> Result<OwnedFd, ShareError> {
    let stat = stat_entry(parent, name)?;
    match FileType::from_raw_mode(stat.st_mode) {
        FileType::Directory => {}
        FileType::Symlink => {
            return Err(ShareError::Refused(format!("{name} 是软链接，核心不跟随")));
        }
        _ => return Err(ShareError::Gone),
    }
    open_checked(parent, name, &stat).map(OwnedFd::from)
}

fn open_regular(dir: BorrowedFd<'_>, name: &str) -> Result<Opened, ShareError> {
    let stat = stat_entry(dir, name)?;
    match FileType::from_raw_mode(stat.st_mode) {
        FileType::RegularFile => {}
        FileType::Symlink => {
            return Err(ShareError::Refused(format!("{name} 是软链接，核心不跟随")));
        }
        _ => return Err(ShareError::Refused(format!("{name} 不是普通文件"))),
    }
    let file = open_checked(dir, name, &stat)?;
    let (snapshot, _) = Snapshot::of(&file).map_err(|e| from_io(&e, name))?;
    Ok(Opened { file, snapshot })
}

fn sync_dir(dir: BorrowedFd<'_>, what: &str) -> Result<(), ShareError> {
    #[cfg(test)]
    if tests::DIR_SYNC_FAILS.with_borrow(|name| name.as_deref() == Some(what)) {
        return Err(classify(Errno::IO, what));
    }
    store::sync_dir(dir).map_err(|e| classify(e, what))
}

/// `a.txt`、`a (1).txt`、`a (2).txt`……
fn numbered(name: &str, attempt: u32) -> String {
    if attempt == 0 {
        return name.to_owned();
    }
    match name.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() => format!("{stem} ({attempt}).{extension}"),
        _ => format!("{name} ({attempt})"),
    }
}

/// 按 Unicode 类别过滤，不列黑名单：格式字符（bidi 覆盖和隔离能把 `exe.jpg` 显示成 `gpj.exe`，零宽字符和标签字符能让两个名字
/// 看着一样）、行分隔符、私有区，再加上 [`default_ignorable`]。
///
/// 这些字符有的有正当用途，故意不按上下文保留：去掉 ZWJ 后 👨‍👩‍👧 拆成三个人，去掉标签字符后苏格兰旗变成黑旗，去掉 ZWNJ 后波斯文的
/// 字形会变。要防的是两个看着一样的名字；变了样的名字还读得懂，主人也能改回来。
fn hides_text(c: char) -> bool {
    matches!(
        c.general_category(),
        GeneralCategory::Control
            | GeneralCategory::Format
            | GeneralCategory::LineSeparator
            | GeneralCategory::ParagraphSeparator
            | GeneralCategory::PrivateUse
    ) || default_ignorable(c)
}

/// Unicode 的 `Default_Ignorable_Code_Point` 里不属于上面那些类别的字符（`unicode-properties` 不提供这个属性）：同样显示不出来。
/// U+FE0F 不算：它单独用时显示得出来（emoji 样式），并且是几乎所有 emoji 序列的一部分，去掉的话 ❤️ 这样的名字都变样。
fn default_ignorable(c: char) -> bool {
    matches!(
        c,
        '\u{034F}'
            | '\u{115F}'..='\u{1160}'
            | '\u{17B4}'..='\u{17B5}'
            | '\u{180B}'..='\u{180F}'
            | '\u{2065}'
            | '\u{3164}'
            | '\u{FE00}'..='\u{FE0E}'
            | '\u{FFA0}'
            | '\u{FFF0}'..='\u{FFF8}'
            | '\u{1BCA0}'..='\u{1BCA3}'
            | '\u{1D173}'..='\u{1D17A}'
            | '\u{E0000}'..='\u{E0FFF}'
    )
}

/// 文件名是一段不可信的文字（客户端声明的、Bot 登记的）：只留最后一段，去掉控制字符和看不见的字符，不占核心的私有名字，太长时
/// 从主名截断、保留扩展名。
pub fn sanitize_name(raw: &str) -> String {
    let base = raw.rsplit(['/', '\\']).next().unwrap_or_default();
    let cleaned: String = base.chars().filter(|c| !hides_text(*c)).collect();
    let cleaned = cleaned.trim();
    if matches!(cleaned, "" | "." | "..") {
        return "file".to_owned();
    }
    let cleaned = if is_private(cleaned) {
        &cleaned[1..]
    } else {
        cleaned
    };
    if cleaned.len() <= MAX_NAME_BYTES {
        return cleaned.to_owned();
    }
    let (stem, extension) = match cleaned.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() && extension.len() <= 16 => (stem, extension),
        _ => (cleaned, ""),
    };
    let room = MAX_NAME_BYTES - extension.len() - 1;
    let end = (0..=room)
        .rev()
        .find(|i| stem.is_char_boundary(*i))
        .unwrap_or(0);
    if extension.is_empty() {
        stem[..end].to_owned()
    } else {
        format!("{}.{extension}", &stem[..end])
    }
}

#[cfg(test)]
pub(super) mod tests;
