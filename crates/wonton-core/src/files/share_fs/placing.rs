//! 把传完的上传放进 `transfers`，以及作废时收拾放下的文件（ADR-0020 的“上传”）。

use std::fs::File;
use std::io::{self, Read};
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::fs::{MetadataExt, fchown};
use std::path::Path;

use rustix::fs::{AtFlags, Dir, Mode, OFlags, linkat, openat, statat, unlinkat};
use rustix::io::Errno;

use super::removal::{remove_entry, remove_if_same};
use super::{
    Deleted, MAX_NAME_TRIES, Opened, ShareError, Shares, Snapshot, TRANSFERS, UPLOAD_TEMP_PREFIX,
    classify, from_io, is_private, is_same_file, numbered, open_absolute, open_regular, stat_entry,
    sync_dir,
};
use crate::l10n;
use crate::plugin::hostdir::Owner;
use crate::plugin::languages::Languages;

/// 作废的上传放下的文件怎么样了（[`Shares::settle_abandoned`]）。
#[derive(Debug, PartialEq, Eq)]
pub enum Abandoned {
    /// 公开的名字上没有它。
    NotPlaced,
    /// 还是放下时的样子，删掉了。`durable` 为假时目录没能确认落盘，断电后文件可能回来。
    Deleted { name: String, durable: bool },
    /// 主人改过，留着。
    Kept { name: String, snapshot: Snapshot },
}

/// 链接之前告诉调用方：要放到这个名字上，文件这时是这个样子。
pub type BeforeLink<'a> = dyn FnMut(&str, &Snapshot) -> Result<(), ShareError> + 'a;

impl Shares {
    /// 把传完的数据（`uploads` 目录下的 `id`）放进 `transfers`，返回最后的名字和文件的样子。数据不搬走：调用方写好记录之后
    /// 再删，这样任何一步失败或者崩溃，再调一次都能收敛到同一个文件（见 [`already_placed`]）。
    ///
    /// tusd 的数据目录归 `PUID`，里面的东西能被换掉，所以相对打开的目录、不跟随软链接地打开数据，之后只认这个 fd。
    ///
    /// 先放到私有的临时名字上（同一个卷上是硬链接，不在同一个卷上是复制），核对是打开时的那个文件，再用 `linkat` 放到目标名上：
    /// 名字已经有东西时报 `EEXIST`，盖不到主人的文件，别的插件也看不到写了一半的文件。放好之后目标名字就是主人的了，核对不符也
    /// 不碰它。
    ///
    /// 每次链接之前先调 `before_link`，告诉调用方要放到哪个名字、文件这时的样子：作废时没有别的办法分清名字上的文件还是放下时的样子，
    /// 还是主人后来改过的（临时名和公开名是同一个 inode，事后取的快照总对得上）。
    pub fn place_upload(
        &self,
        uploads: &Path,
        id: &str,
        size: u64,
        names: Names<'_>,
        langs: &Languages,
        before_link: &mut BeforeLink<'_>,
    ) -> Result<(String, Snapshot), ShareError> {
        let uploads = open_absolute(uploads, "上传目录")?;
        let source = open_regular(uploads.as_fd(), id)?;
        if source.snapshot.size != size {
            return Err(ShareError::Io(l10n::text(
                langs,
                "files-size-mismatch",
                &[
                    ("actual", wonton_l10n::Arg::from(source.snapshot.size)),
                    ("size", wonton_l10n::Arg::from(size)),
                ],
            )));
        }
        let dir = self.open_dir(TRANSFERS, &[])?;
        if let Some(name) = already_placed(dir.as_fd(), &source.file)? {
            return placed(dir.as_fd(), name, &source.file, names.owner);
        }
        source.file.sync_all().map_err(|e| from_io(&e, id))?;
        through_temp(
            &source,
            id,
            dir.as_fd(),
            names,
            langs,
            before_link,
            |temp| link_to_temp(uploads.as_fd(), id, dir.as_fd(), temp, &source.file, langs),
        )
    }

    /// 上传作废时，这个上传放下的文件怎么办。`placing` 是 `before_link` 最后一次记下的名字和样子。名字上还是放下时的样子，就按没放过算，
    /// 删掉：上传从没报告过成功。主人改过（改了名、改了内容），它就是主人的文件，留着，调用方补记录。名字上是不相干的文件，又找不到
    /// 这个 inode（主人移走了、覆盖了、或者崩溃在链接之前）：不动，也不补记录，那不是这个上传的文件。临时文件的名字插件也写得到，所以
    /// 要是记下的那个 inode 才算（见 [`earlier_temp`]）。
    pub fn settle_abandoned(
        &self,
        id: &str,
        placing: Option<(&str, &Snapshot)>,
        langs: &Languages,
    ) -> Result<Abandoned, ShareError> {
        let Some((name, at_placing)) = placing else {
            return Ok(Abandoned::NotPlaced);
        };
        let dir = match self.open_dir(TRANSFERS, &[]) {
            Ok(dir) => dir,
            Err(ShareError::Gone) => return Ok(Abandoned::NotPlaced),
            Err(e) => return Err(e),
        };
        let unchanged = match open_regular(dir.as_fd(), name) {
            Ok(opened) => opened.snapshot == *at_placing,
            Err(ShareError::Gone | ShareError::Refused(_)) => false,
            Err(e) => return Err(e),
        };
        if unchanged {
            let _removing = self.removing.lock().expect("删除锁中毒");
            match remove_if_same(dir.as_fd(), dir.as_fd(), name, at_placing, langs)? {
                Deleted::Removed { durable } => {
                    return Ok(Abandoned::Deleted {
                        name: name.to_owned(),
                        durable,
                    });
                }
                // 核对之后才被改了：接着往下，按主人改过的算。
                Deleted::AlreadyGone | Deleted::Changed => {}
            }
        }
        let temp = format!("{UPLOAD_TEMP_PREFIX}{id}");
        let opened = match open_regular(dir.as_fd(), &temp) {
            Ok(opened) => opened,
            Err(ShareError::Gone | ShareError::Refused(_)) => return Ok(Abandoned::NotPlaced),
            Err(e) => return Err(e),
        };
        if opened.snapshot.inode != at_placing.inode {
            return Ok(Abandoned::NotPlaced);
        }
        Ok(match already_placed(dir.as_fd(), &opened.file)? {
            Some(name) => Abandoned::Kept {
                name,
                snapshot: opened.snapshot,
            },
            None => Abandoned::NotPlaced,
        })
    }

    /// 上传结束后删掉临时文件。
    pub fn remove_upload_temp(&self, id: &str) -> Result<(), ShareError> {
        let dir = match self.open_dir(TRANSFERS, &[]) {
            Ok(dir) => dir,
            Err(ShareError::Gone) => return Ok(()),
            Err(e) => return Err(e),
        };
        let temp = format!("{UPLOAD_TEMP_PREFIX}{id}");
        remove_entry(dir.as_fd(), &temp)?;
        sync_dir(dir.as_fd(), &temp)
    }

    /// `transfers` 里的上传临时文件，去掉前缀之后的名字。
    pub fn upload_temps(&self) -> Result<Vec<String>, ShareError> {
        let dir = self.open_dir(TRANSFERS, &[])?;
        let mut temps = Vec::new();
        for entry in Dir::read_from(&dir).map_err(|e| classify(e, TRANSFERS))? {
            let entry = entry.map_err(|e| classify(e, TRANSFERS))?;
            if let Some(id) = entry
                .file_name()
                .to_str()
                .ok()
                .and_then(|name| name.strip_prefix(UPLOAD_TEMP_PREFIX))
            {
                temps.push(id.to_owned());
            }
        }
        Ok(temps)
    }
}

fn no_hard_links(name: &str, langs: &Languages) -> ShareError {
    ShareError::Io(l10n::text(
        langs,
        "files-no-hard-links",
        &[("name", name.into()), ("folder", TRANSFERS.into())],
    ))
}

/// 把 `from` 链接到第一个空着的名字（重名时加序号）。软链接也算重名。已经有一个名字就是它（上次在链接之后崩溃了）就用那个名字，
/// 同一个文件不出现两个名字。查过名字空着，调用方记下了之后才链接；`EEXIST` 说明这之间被占了，换下一个再记一次。
fn pick_name(
    dir: BorrowedFd<'_>,
    from: &str,
    preferred: &str,
    file: &File,
    langs: &Languages,
    before_link: &mut BeforeLink<'_>,
) -> Result<String, ShareError> {
    let source = stat_entry(dir, from)?;
    let same = |name: &str| {
        stat_entry(dir, name)
            .is_ok_and(|taken| taken.st_ino == source.st_ino && taken.st_dev == source.st_dev)
    };
    let (snapshot, _) = Snapshot::of(file).map_err(|e| from_io(&e, from))?;
    for attempt in 0..MAX_NAME_TRIES {
        let name = numbered(preferred, attempt);
        match stat_entry(dir, &name) {
            Ok(_) if same(&name) => return Ok(name),
            Ok(_) => continue,
            Err(ShareError::Gone) => {}
            Err(e) => return Err(e),
        }
        before_link(&name, &snapshot)?;
        #[cfg(test)]
        if super::tests::LINK_FAILS
            .lock()
            .expect("链接替身锁中毒")
            .as_deref()
            == Some(from.strip_prefix(UPLOAD_TEMP_PREFIX).unwrap_or(from))
        {
            super::tests::LINK_FAILS.lock().unwrap().take();
            return Err(ShareError::Io(format!("{name} 刚被换掉了")));
        }
        match linkat(dir, from, dir, name.as_str(), AtFlags::empty()) {
            Ok(()) => return Ok(name),
            Err(Errno::EXIST) if same(&name) => return Ok(name),
            Err(Errno::EXIST) => {}
            Err(Errno::PERM | Errno::NOTSUP | Errno::MLINK) => {
                return Err(no_hard_links(&name, langs));
            }
            Err(e) => return Err(classify(e, &name)),
        }
    }
    Err(ShareError::Io(l10n::text(
        langs,
        "files-name-taken",
        &[
            ("preferred", preferred.into()),
            ("count", wonton_l10n::Arg::from(MAX_NAME_TRIES as i64)),
        ],
    )))
}

/// [`pick_name`]，再核对名字上的还是 `file`。不是的话那已经是别人（Samba 另存、插件）的文件：报错，不删。
pub(super) fn link_checked(
    dir: BorrowedFd<'_>,
    from: &str,
    preferred: &str,
    file: &File,
    langs: &Languages,
    before_link: &mut BeforeLink<'_>,
) -> Result<String, ShareError> {
    let name = pick_name(dir, from, preferred, file, langs, before_link)?;
    let meta = file.metadata().map_err(|e| from_io(&e, &name))?;
    if !is_same_file(&stat_entry(dir, &name)?, &meta) {
        return Err(ShareError::Refused(format!("{name} 刚被换掉了")));
    }
    Ok(name)
}

/// 同一个卷上：把数据链接成临时文件，用打开时的 fd 核对，`linkat` 按名字找，这之间名字可能被换掉。不在同一个卷上时为空。
pub(super) fn link_to_temp(
    data_dir: BorrowedFd<'_>,
    id: &str,
    dir: BorrowedFd<'_>,
    temp: &str,
    source: &File,
    langs: &Languages,
) -> Result<Option<File>, ShareError> {
    #[cfg(test)]
    if super::tests::CROSS_VOLUME
        .lock()
        .expect("跨卷替身锁中毒")
        .as_deref()
        == Some(id)
    {
        return Ok(None);
    }
    match linkat(data_dir, id, dir, temp, AtFlags::empty()) {
        Ok(()) => {}
        Err(Errno::XDEV) => return Ok(None),
        Err(Errno::PERM | Errno::NOTSUP | Errno::MLINK) => return Err(no_hard_links(temp, langs)),
        Err(e) => return Err(classify(e, temp)),
    }
    let meta = source.metadata().map_err(|e| from_io(&e, id))?;
    if !is_same_file(&stat_entry(dir, temp)?, &meta) {
        unlinkat(dir, temp, AtFlags::empty()).map_err(|e| classify(e, temp))?;
        return Err(ShareError::Refused(format!("{id} 刚被换掉了")));
    }
    source.try_clone().map(Some).map_err(|e| from_io(&e, id))
}

/// 放到什么名字上、归谁。
#[derive(Clone, Copy)]
pub struct Names<'a> {
    pub preferred: &'a str,
    pub owner: Owner,
    /// 上次走到链接这一步时（`before_link` 记下的）文件的 inode。
    pub placing_inode: Option<u64>,
}

/// 经临时文件放到目标名上。`link` 准备临时文件，不在同一个卷上时给不出，就复制一份。
pub(super) fn through_temp(
    source: &Opened,
    id: &str,
    dir: BorrowedFd<'_>,
    Names {
        preferred,
        owner,
        placing_inode,
    }: Names<'_>,
    langs: &Languages,
    before_link: &mut BeforeLink<'_>,
    link: impl FnOnce(&str) -> Result<Option<File>, ShareError>,
) -> Result<(String, Snapshot), ShareError> {
    let temp = format!("{UPLOAD_TEMP_PREFIX}{id}");
    if let Some((name, file)) = earlier_temp(dir, &temp, placing_inode)? {
        return placed(dir, name, &file, owner);
    }
    let file = match link(&temp)? {
        Some(file) => file,
        None => copy_to_temp(source, dir, &temp, owner, langs)?,
    };
    match link_checked(dir, &temp, preferred, &file, langs, before_link) {
        Ok(name) => placed(dir, name, &file, owner),
        Err(e) => {
            remove_temp(dir, &temp, &file);
            Err(e)
        }
    }
}

/// 删掉没用上的临时文件（可能很大），名字上还是它才删。
fn remove_temp(dir: BorrowedFd<'_>, temp: &str, file: &File) {
    let ours = match (stat_entry(dir, temp), file.metadata()) {
        (Ok(stat), Ok(meta)) => is_same_file(&stat, &meta),
        _ => false,
    };
    if ours && let Err(error) = unlinkat(dir, temp, AtFlags::empty()) {
        tracing::warn!(name = temp, %error, "删除没用上的临时文件失败");
    }
}

/// 上次留下的临时文件。已经放好了（崩溃在写记录之前）就给出它放到的名字和文件。否则删掉重来：可能是半个文件，也可能放好之后
/// 又被主人移走了。这个名字插件也写得到：它可以自己建一个，再硬链接到公开的名字上，所以要是上次记下的那个 inode，否则插件的内容就算成了
/// 设备上传。半个文件的 inode 不会被记下，插件链接它也没用。不比大小：放好之后主人可以改内容，同一个卷上的路径也不比。不比设备号：同一个
/// 目录里的硬链接不跨设备，而设备号重启后可能变（LVM、NFS），比它只会把放好的文件当成别人的。
fn earlier_temp(
    dir: BorrowedFd<'_>,
    temp: &str,
    placing_inode: Option<u64>,
) -> Result<Option<(String, File)>, ShareError> {
    let old = match open_regular(dir, temp) {
        Ok(old) => old,
        Err(ShareError::Gone) => return Ok(None),
        // 这个名字只属于核心，上面不是普通文件就是别人放的：清掉，否则这个上传永远放不进去。
        Err(ShareError::Refused(_)) => {
            remove_entry(dir, temp)?;
            return Ok(None);
        }
        Err(e) => return Err(e),
    };
    if placing_inode == Some(old.snapshot.inode)
        && let Some(name) = already_placed(dir, &old.file)?
    {
        return Ok(Some((name, old.file)));
    }
    unlinkat(dir, temp, AtFlags::empty()).map_err(|e| classify(e, temp))?;
    Ok(None)
}

/// 这个文件已经链接到 `transfers` 里主人看得到的名字上了吗（放好之后、写好记录之前失败或者崩溃过）。只有链接数大于 1 才翻
/// 目录。主人这时改了名也认得出；移走了或者删了就算没放过，再放一次。
fn already_placed(dir: BorrowedFd<'_>, file: &File) -> Result<Option<String>, ShareError> {
    let meta = file.metadata().map_err(|e| from_io(&e, "传完的文件"))?;
    if meta.nlink() < 2 {
        return Ok(None);
    }
    let entries = Dir::read_from(dir).map_err(|e| classify(e, TRANSFERS))?;
    for entry in entries {
        let entry = entry.map_err(|e| classify(e, TRANSFERS))?;
        let Ok(name) = entry.file_name().to_str() else {
            continue;
        };
        if matches!(name, "." | "..") || is_private(name) {
            continue;
        }
        let Ok(stat) = statat(dir, name, AtFlags::SYMLINK_NOFOLLOW) else {
            continue;
        };
        if is_same_file(&stat, &meta) {
            return Ok(Some(name.to_owned()));
        }
    }
    Ok(None)
}

/// 文件已经在目标名字上了：属主、快照都用这个 fd，不按名字再开一次，然后把目录项落盘。
fn placed(
    dir: BorrowedFd<'_>,
    name: String,
    file: &File,
    owner: Owner,
) -> Result<(String, Snapshot), ShareError> {
    give_to(file, owner).map_err(|e| from_io(&e, &name))?;
    let (snapshot, _) = Snapshot::of(file).map_err(|e| from_io(&e, &name))?;
    sync_dir(dir, &name)?;
    Ok((name, snapshot))
}

/// 不在同一个卷上：复制成临时文件，写完落盘。失败时删掉，不占着空间。
fn copy_to_temp(
    source: &Opened,
    dir: BorrowedFd<'_>,
    temp: &str,
    owner: Owner,
    langs: &Languages,
) -> Result<File, ShareError> {
    let flags = OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let fd = openat(dir, temp, flags, Mode::from_raw_mode(0o664)).map_err(|e| classify(e, temp))?;
    let mut target = File::from(fd);
    let size = source.snapshot.size;
    let copied = (|| -> io::Result<u64> {
        let mut from = (&source.file).take(size);
        let copied = io::copy(&mut from, &mut target)?;
        give_to(&target, owner)?;
        target.sync_all()?;
        Ok(copied)
    })();
    match copied {
        Ok(copied) if copied == size => Ok(target),
        Ok(copied) => {
            remove_temp(dir, temp, &target);
            Err(ShareError::Io(l10n::text(
                langs,
                "files-short-read",
                &[
                    ("copied", wonton_l10n::Arg::from(copied)),
                    ("size", wonton_l10n::Arg::from(size)),
                ],
            )))
        }
        Err(e) => {
            remove_temp(dir, temp, &target);
            Err(from_io(&e, temp))
        }
    }
}

/// 属主已经对了就不改：核心不以 root 运行时也能工作。
pub(super) fn give_to(file: &File, owner: Owner) -> io::Result<()> {
    let meta = file.metadata()?;
    if meta.uid() == owner.uid && meta.gid() == owner.gid {
        return Ok(());
    }
    fchown(file, Some(owner.uid), Some(owner.gid))
}
