//! 从 `transfers` 里删东西：核对后删文件、崩溃后找回、清掉核心私有名字上的东西。
//!
//! 删除先把文件改名到预先建好的回收槽（`.wonton-trash/<n>/<原名>`）。原名就在目录项里，“记下原名”和“改名”是同一次原子操作，
//! 没有崩溃窗口；整个过程只动元数据，所以磁盘写满时也能删，删文件正是腾空间的办法。
//!
//! 空目录不一定有块：XFS 把小目录的项放在 inode 里，放不下时要新块，写满时长名字就改不进去（实测见 ADR-0020 的“验证结果”）。所以每个槽
//! 放一个名字最长的占位文件（[`keep_name`]），目录一直有块。
//!
//! `transfers` 归 `PUID`，有写权限的插件进不了 0700 的回收槽，却能把 `.wonton-trash` 整个改名，自己再建一套。所以每次删除都核对打开的
//! 目录（[`open_private`]）：运行中不建、不修，对不上就如实报，重启时才重建。核心不以 root 运行时属主和插件相同，这一条核对不出来，靠取出
//! 来的名字再核对一次（[`remove_if_same`]）。

use std::os::fd::{AsFd, BorrowedFd, OwnedFd};

use rustix::fs::{
    AtFlags, Dir, FileType, Mode, OFlags, RenameFlags, Stat, fstat, mkdirat, openat, renameat,
    renameat_with, statat, unlinkat,
};
use rustix::io::Errno;
use rustix::process::geteuid;

use super::{
    Deleted, MAX_NAME_TRIES, PRIVATE_PREFIX, ShareError, Snapshot, TRANSFERS, check_segment,
    classify, is_private, numbered, open_regular, open_subdir, stat_entry, sync_dir,
};

/// 回收槽的上级目录，在 `transfers` 里：改名不能跨文件系统。
pub(super) const TRASH: &str = ".wonton-trash";
/// 回收槽的个数。删除一个一个来（[`super::Shares`] 的锁），多个槽是为了一个槽清不空（放不回去）时别的删除照常。
const TRASH_SLOTS: usize = 8;

/// 建好全部回收槽：要在磁盘满之前建好。`.wonton-` 开头的名字由核心保留，上面放的不是核心的回收槽就清掉，否则一个插件就能让核心
/// 起不来；里面的文件先放回去（[`recover_foreign`]），还有东西的目录改名交还给主人（[`remove_entry`]）。建好了还对不上，是文件系统不
/// 保留属主和权限，保证不了只有核心碰得到，启动失败。
pub(super) fn prepare_trash(transfers: BorrowedFd<'_>) -> Result<(), ShareError> {
    if foreign_trash(transfers)? {
        tracing::warn!("{TRASH} 不是核心建的回收槽，挪开重建");
        recover_foreign(transfers);
        remove_entry(transfers, TRASH)?;
    }
    make_dir(transfers, TRASH)?;
    let trash = just_made(transfers, TRASH)?;
    let keep = keep_name();
    for slot in 0..TRASH_SLOTS {
        let slot = slot.to_string();
        make_dir(trash.as_fd(), &slot)?;
        let slot = just_made(trash.as_fd(), &slot)?;
        let flags = OFlags::CREATE | OFlags::EXCL | OFlags::WRONLY | OFlags::CLOEXEC;
        match openat(slot.as_fd(), &keep, flags, Mode::from_raw_mode(0o600)) {
            // 名字这么长都建不了的文件系统（eCryptfs），主人的文件名也到不了这么长。
            Ok(_) | Err(Errno::EXIST | Errno::NAMETOOLONG) => {}
            Err(e) => return Err(classify(e, TRASH)),
        }
        sync_dir(slot.as_fd(), TRASH)?;
    }
    sync_dir(trash.as_fd(), TRASH)?;
    sync_dir(transfers, TRASH)
}

/// 看着不像核心建的回收槽里，也可能有删除到一半的文件：NAS 的“重置权限”会把真的回收槽改成这样。这里先放回去，否则交还之后就回不到
/// 原名了。能打开多少算多少，别的跟着目录交还。名字可能是插件放的，[`recover_one`] 照样核对。
fn recover_foreign(transfers: BorrowedFd<'_>) {
    let Ok(trash) = open_subdir(transfers, TRASH) else {
        return;
    };
    let keep = keep_name();
    for index in 0..TRASH_SLOTS {
        let name = index.to_string();
        let Ok(slot) = open_subdir(trash.as_fd(), &name) else {
            continue;
        };
        let Ok(entries) = entries_of(slot.as_fd(), TRASH) else {
            continue;
        };
        for entry in entries {
            match recover_one(slot.as_fd(), &entry, transfers) {
                Ok(restored) => tracing::warn!(name = restored, "找回了删除到一半的文件"),
                Err(error) => tracing::error!(name = entry, %error, "找回删除到一半的文件失败"),
            }
        }
        // 只剩占位文件的槽不用交还。清不掉就跟着目录走。
        let _ = unlinkat(slot.as_fd(), keep.as_str(), AtFlags::empty());
        let _ = unlinkat(trash.as_fd(), name.as_str(), AtFlags::REMOVEDIR);
    }
    if let Err(error) = sync_dir(transfers, TRASH) {
        tracing::warn!(%error, "放回去的文件没能确认落盘");
    }
}

fn just_made(parent: BorrowedFd<'_>, name: &str) -> Result<OwnedFd, ShareError> {
    open_private(parent, name).map_err(|e| {
        ShareError::Io(format!(
            "{e}。{TRANSFERS} 所在的文件系统可能不保留属主和权限（CIFS、root_squash 的 NFS）"
        ))
    })
}

/// `.wonton-trash` 上有东西，且它或者里面的某个槽不是核心建的。
fn foreign_trash(transfers: BorrowedFd<'_>) -> Result<bool, ShareError> {
    let euid = geteuid().as_raw();
    match stat_entry(transfers, TRASH) {
        Ok(stat) if !is_ours(&stat, euid) => return Ok(true),
        Ok(_) => {}
        Err(ShareError::Gone) => return Ok(false),
        Err(e) => return Err(e),
    }
    let trash = open_subdir(transfers, TRASH)?;
    for slot in 0..TRASH_SLOTS {
        match stat_entry(trash.as_fd(), &slot.to_string()) {
            Ok(stat) if !is_ours(&stat, euid) => return Ok(true),
            Ok(_) | Err(ShareError::Gone) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(false)
}

/// 核心建的回收槽：目录，属主是核心，只有属主碰得到。不看 setgid：上级目录有它时新目录会继承。
pub(super) fn is_ours(stat: &Stat, euid: u32) -> bool {
    FileType::from_raw_mode(stat.st_mode) == FileType::Directory
        && stat.st_uid == euid
        && stat.st_mode & 0o777 == 0o700
}

/// 槽里的占位文件的名字，文件名的最大长度。
pub(super) fn keep_name() -> String {
    format!(
        "{PRIVATE_PREFIX}keep-{}",
        "k".repeat(255 - PRIVATE_PREFIX.len() - 5)
    )
}

fn open_trash(transfers: BorrowedFd<'_>) -> Result<OwnedFd, ShareError> {
    open_private(transfers, TRASH)
}

fn open_slot(trash: BorrowedFd<'_>, slot: usize) -> Result<OwnedFd, ShareError> {
    open_private(trash, &slot.to_string())
}

/// 打开回收槽或者它的上级，核对打开的目录，这之后名字再被换也和它无关。
fn open_private(parent: BorrowedFd<'_>, name: &str) -> Result<OwnedFd, ShareError> {
    let dir = open_subdir(parent, name).map_err(|e| match e {
        ShareError::Gone | ShareError::Refused(_) => {
            ShareError::Io(format!("回收槽 {name} 不见了，要重启核心才能再删"))
        }
        other => other,
    })?;
    let stat = fstat(&dir).map_err(|e| classify(e, name))?;
    let euid = geteuid().as_raw();
    if !is_ours(&stat, euid) {
        return Err(ShareError::Io(format!(
            "回收槽 {name} 不是核心建的（属主 {}、权限 {:o}，应该是 {euid}、700），要重启核心才能再删",
            stat.st_uid,
            stat.st_mode & 0o777,
        )));
    }
    Ok(dir)
}

/// 没有占位文件的槽，磁盘写满时可能改不进长名字。名字这么长都放不下的文件系统本来就没有它。
fn check_placeholder(slot: BorrowedFd<'_>, index: usize) -> Result<(), ShareError> {
    match statat(slot, keep_name(), AtFlags::SYMLINK_NOFOLLOW) {
        Ok(_) | Err(Errno::NAMETOOLONG) => Ok(()),
        Err(Errno::NOENT) => Err(ShareError::Io(format!(
            "回收槽 {index} 的占位文件不见了，要重启核心才能再删"
        ))),
        Err(e) => Err(classify(e, TRASH)),
    }
}

fn make_dir(parent: BorrowedFd<'_>, name: &str) -> Result<(), ShareError> {
    match mkdirat(parent, name, Mode::from_raw_mode(0o700)) {
        Ok(()) | Err(Errno::EXIST) => Ok(()),
        Err(e) => Err(classify(e, name)),
    }
}

/// 这个槽里的名字，占位文件和名字不是 UTF-8 的除外（核心只放过 UTF-8 的名字）。
fn entries_of(dir: BorrowedFd<'_>, what: &str) -> Result<Vec<String>, ShareError> {
    let keep = keep_name();
    let mut names = Vec::new();
    for entry in Dir::read_from(dir).map_err(|e| classify(e, what))? {
        let entry = entry.map_err(|e| classify(e, what))?;
        let raw = entry.file_name().to_bytes();
        if matches!(raw, b"." | b"..") {
            continue;
        }
        match entry.file_name().to_str() {
            Ok(name) if name == keep => {}
            Ok(name) => names.push(name.to_owned()),
            Err(_) => tracing::warn!(dir = what, "回收槽里有名字不是 UTF-8 的东西，没有动它"),
        }
    }
    Ok(names)
}

/// 一个空的回收槽。调用方持有 [`super::Shares`] 的删除锁。
fn empty_slot(transfers: BorrowedFd<'_>) -> Result<OwnedFd, ShareError> {
    let trash = open_trash(transfers)?;
    for index in 0..TRASH_SLOTS {
        let slot = open_slot(trash.as_fd(), index)?;
        check_placeholder(slot.as_fd(), index)?;
        if entries_of(slot.as_fd(), TRASH)?.is_empty() {
            return Ok(slot);
        }
    }
    Err(ShareError::Io(format!(
        "{TRASH} 里的回收槽都被占着，重启核心找回里面的文件后才能再删"
    )))
}

/// 把 `name` 改名到回收槽，这个名字上是谁就定下来了，再核对、再删；不是要删的文件就放回去。`dir` 是 `name` 所在的目录。
pub(super) fn remove_if_same(
    transfers: BorrowedFd<'_>,
    dir: BorrowedFd<'_>,
    name: &str,
    expected: &Snapshot,
) -> Result<Deleted, ShareError> {
    let slot = empty_slot(transfers)?;
    let slot = slot.as_fd();
    match rename_no_replace(dir, name, slot, name) {
        Ok(()) => {}
        // 目标所在的槽也可能不在了：名字还在就不是“已经没了”。
        Err(Errno::NOENT) if matches!(stat_entry(dir, name), Err(ShareError::Gone)) => {
            return Ok(Deleted::AlreadyGone);
        }
        Err(e) => return Err(classify(e, name)),
    }
    let same = match open_regular(slot, name) {
        Ok(opened) => opened.snapshot == *expected,
        Err(ShareError::Refused(_)) => false,
        Err(e) => {
            put_back(slot, name, dir, name)?;
            return Err(e);
        }
    };
    if !same {
        put_back(slot, name, dir, name)?;
        return Ok(Deleted::Changed);
    }
    if let Err(e) = unlinkat(slot, name, AtFlags::empty()) {
        put_back(slot, name, dir, name)?;
        return Err(classify(e, name));
    }
    // 文件已经删了：这里报错的话，调用方会当成没删，审计和记录都错。
    let durable = [slot, dir].into_iter().fold(true, |durable, changed| {
        let synced = sync_dir(changed, name);
        if let Err(error) = &synced {
            tracing::warn!(name, %error, "删除没能确认落盘，断电后文件可能回来");
        }
        durable && synced.is_ok()
    });
    Ok(Deleted::Removed { durable })
}

/// 启动时：改名到回收槽之后崩溃了的文件放回去，它们不放回去就一直藏着。一项失败只记日志，下次启动再试：插件也可能往这里放东西，
/// 不能挡住别的文件。
pub(super) fn recover_trash(transfers: BorrowedFd<'_>) -> Result<(), ShareError> {
    let trash = open_trash(transfers)?;
    for index in 0..TRASH_SLOTS {
        let slot = open_slot(trash.as_fd(), index)?;
        for name in entries_of(slot.as_fd(), TRASH)? {
            match recover_one(slot.as_fd(), &name, transfers) {
                Ok(restored) => tracing::warn!(name = restored, "找回了删除到一半的文件"),
                Err(error) => {
                    tracing::error!(name, %error, "找回删除到一半的文件失败，下次启动再试");
                }
            }
        }
    }
    sync_dir(transfers, TRASH)
}

/// 槽里的名字可能是插件放的：和客户端给的名字一样核对，放回去只会是 `transfers` 里的一个普通名字，占不了核心的私有名字。
fn recover_one(
    slot: BorrowedFd<'_>,
    name: &str,
    transfers: BorrowedFd<'_>,
) -> Result<String, ShareError> {
    check_segment(name).map_err(ShareError::Refused)?;
    let target = if is_private(name) { &name[1..] } else { name };
    put_back(slot, name, transfers, target)
}

/// 放回 `to_dir` 的 `preferred`，返回最后的名字。这期间那个名字可能又有了别的东西：加序号，谁都不盖。
pub(super) fn put_back(
    from_dir: BorrowedFd<'_>,
    from: &str,
    to_dir: BorrowedFd<'_>,
    preferred: &str,
) -> Result<String, ShareError> {
    for attempt in 0..MAX_NAME_TRIES {
        let target = numbered(preferred, attempt);
        match rename_no_replace(from_dir, from, to_dir, &target) {
            Ok(()) => return Ok(target),
            Err(Errno::EXIST | Errno::NOTEMPTY) => {}
            Err(e) => return Err(classify(e, &target)),
        }
    }
    Err(ShareError::Io(format!(
        "{preferred} 已经有 {MAX_NAME_TRIES} 个重名的文件"
    )))
}

/// 不覆盖任何东西的改名：普通的 `rename` 会悄悄盖掉文件和空目录。内核太老（`ENOSYS`）或者文件系统不支持（NFS、CIFS 的
/// `EINVAL`，部分系统的 `ENOTSUP`）时退回 [`rename_if_free`]，否则目录永远放不回去。
fn rename_no_replace(
    from_dir: BorrowedFd<'_>,
    from: &str,
    to_dir: BorrowedFd<'_>,
    to: &str,
) -> Result<(), Errno> {
    let renamed = {
        #[cfg(test)]
        if super::tests::NO_NOREPLACE.get() {
            Err(Errno::INVAL)
        } else {
            renameat_with(from_dir, from, to_dir, to, RenameFlags::NOREPLACE)
        }
        #[cfg(not(test))]
        renameat_with(from_dir, from, to_dir, to, RenameFlags::NOREPLACE)
    };
    match renamed {
        Err(Errno::NOSYS | Errno::INVAL | Errno::NOTSUP) => {
            rename_if_free(from_dir, from, to_dir, to)
        }
        renamed => renamed,
    }
}

/// 先确认目标空着，再普通地改名。查和改之间有一个很小的窗口，只在不支持 `RENAME_NOREPLACE` 的文件系统上才有。
fn rename_if_free(
    from_dir: BorrowedFd<'_>,
    from: &str,
    to_dir: BorrowedFd<'_>,
    to: &str,
) -> Result<(), Errno> {
    match statat(to_dir, to, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(_) => Err(Errno::EXIST),
        Err(Errno::NOENT) => renameat(from_dir, from, to_dir, to),
        Err(e) => Err(e),
    }
}

/// 清掉核心私有名字上的东西。核心自己不在这些名字上建目录，那里的目录只可能是插件放的，可能是它把别人的目录改名过来的，而核心可能
/// 以 root 运行，所以不递归：空目录直接删，非空的改名成看得见的名字交还给主人。
pub(super) fn remove_entry(dir: BorrowedFd<'_>, name: &str) -> Result<(), ShareError> {
    let stat = match stat_entry(dir, name) {
        Ok(stat) => stat,
        Err(ShareError::Gone) => return Ok(()),
        Err(e) => return Err(e),
    };
    let stuck = |e: ShareError| ShareError::Io(format!("私有名字 {name} 被占着、清不掉：{e}"));
    if FileType::from_raw_mode(stat.st_mode) != FileType::Directory {
        return match unlinkat(dir, name, AtFlags::empty()) {
            Ok(()) | Err(Errno::NOENT) => Ok(()),
            Err(e) => Err(stuck(classify(e, name))),
        };
    }
    match unlinkat(dir, name, AtFlags::REMOVEDIR) {
        Ok(()) | Err(Errno::NOENT) => Ok(()),
        Err(Errno::NOTEMPTY | Errno::EXIST) => {
            let visible = name.trim_start_matches('.');
            let returned = put_back(dir, name, dir, visible).map_err(stuck)?;
            tracing::warn!(
                name,
                returned,
                "核心的私有名字上有一个非空目录，改了名交还给主人"
            );
            Ok(())
        }
        Err(e) => Err(stuck(classify(e, name))),
    }
}
