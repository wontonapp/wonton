//! 放不放得下：磁盘空间、inode 和用户配额。配额要经目录 fd 问内核，`statvfs` 看不到（ADR-0020 的“代价与风险”）。

use std::fs;
use std::os::fd::AsRawFd;
use std::path::Path;

use super::KEEP_FREE_PERCENT;
use crate::files::Failure;

/// 文件系统给 `uid` 留的配额。`None` 表示没有用户配额，或者内核、文件系统不支持按目录查（内核早于 5.16）；这时只能看 `statvfs`。
/// 配额满时 `statvfs` 的剩余空间还是很大，看不出 `EDQUOT`，tusd 写字节也只回 500。
#[derive(Debug, Clone, Copy)]
pub(in crate::files) struct Quota {
    /// 生效的配额上限：算要留的空闲和剩余都用它。宽限期里软限不算，按硬限；两个都是 0 就是没设限。
    pub(in crate::files) space_limit: u64,
    pub(in crate::files) space_left: Option<u64>,
    pub(in crate::files) inodes_left: Option<u64>,
}

pub(in crate::files) fn quota(dir: &Path, uid: u32) -> Option<Quota> {
    #[cfg(test)]
    if let Some((_, quota)) = super::quota_tests::QUOTA
        .lock()
        .expect("配额替身锁中毒")
        .iter()
        .find(|(path, _)| path == dir)
    {
        return Some(*quota);
    }
    let fd = fs::File::open(dir).ok()?;
    let mut block: libc::dqblk = unsafe { std::mem::zeroed() };
    let asked = unsafe {
        libc::syscall(
            libc::SYS_quotactl_fd,
            fd.as_raw_fd(),
            libc::QCMD(libc::Q_GETQUOTA, types::USRQUOTA),
            uid,
            &mut block as *mut libc::dqblk,
        )
    };
    if asked != 0 {
        return None;
    }
    let valid = block.dqb_valid;
    let space = valid & types::QIF_BLIMITS != 0 && valid & types::QIF_SPACE != 0;
    let space_limit = [block.dqb_bhardlimit, block.dqb_bsoftlimit]
        .into_iter()
        .filter(|limit| *limit > 0)
        .min()
        .unwrap_or(0);
    // 软限的宽限期内（`dqb_btime` 大于 0）还能写，按硬限算；宽限过了软限就是硬限。
    let space_limit = if block.dqb_btime > 0 {
        block.dqb_bhardlimit
    } else {
        space_limit
    };
    let inodes = valid & types::QIF_ILIMITS != 0 && valid & types::QIF_INODES != 0;
    let inodes_limit = [block.dqb_ihardlimit, block.dqb_isoftlimit]
        .into_iter()
        .filter(|limit| *limit > 0)
        .min()
        .unwrap_or(0);
    let inodes_limit = if block.dqb_itime > 0 {
        block.dqb_ihardlimit
    } else {
        inodes_limit
    };
    // 块上限的单位是 1 KiB，`dqb_curspace` 是字节。上限为 0 就是没设限：启用了配额但没给这个 uid 设限时内核也回两个 0。
    let space_limit = space_limit.saturating_mul(1024);
    Some(Quota {
        space_limit,
        space_left: (space && space_limit > 0)
            .then(|| space_limit.saturating_sub(block.dqb_curspace)),
        inodes_left: (inodes && inodes_limit > 0)
            .then(|| inodes_limit.saturating_sub(block.dqb_curinodes)),
    })
}

/// `linux/quota.h` 里没被 `libc` 带出来的常量。
mod types {
    pub(super) const USRQUOTA: libc::c_int = 0;
    pub(super) const QIF_BLIMITS: u32 = 0x2;
    pub(super) const QIF_SPACE: u32 = 0x1;
    pub(super) const QIF_ILIMITS: u32 = 0x8;
    pub(super) const QIF_INODES: u32 = 0x4;
}

/// 这个目录所在的磁盘放下 `size` 字节后，还空着 [`KEEP_FREE_PERCENT`]；有用户配额时两个限制都要满足。
pub(in crate::files) fn has_room(dir: &Path, size: u64, uid: u32) -> Result<bool, Failure> {
    let stats = rustix::fs::statvfs(dir)
        .map_err(|e| Failure::internal(format!("查 {} 的磁盘空间失败：{e}", dir.display())))?;
    if let Some(quota) = quota(dir, uid) {
        if let Some(left) = quota.space_left
            && left < size.saturating_add(quota.space_limit / 100 * KEEP_FREE_PERCENT)
        {
            return Ok(false);
        }
        if quota.inodes_left == Some(0) {
            return Ok(false);
        }
    }
    let frsize = stats.f_frsize.max(1);
    let free = stats.f_bavail.saturating_mul(frsize);
    let keep = stats.f_blocks.saturating_mul(frsize) / 100 * KEEP_FREE_PERCENT;
    let enough = size.checked_add(keep).is_some_and(|needed| needed <= free);
    Ok(enough && !out_of_inodes(stats.f_files, stats.f_favail))
}

/// inode 用完了也放不下。总数是 0 表示这个文件系统没有这个限制（btrfs）。
pub(in crate::files) fn out_of_inodes(total: u64, free: u64) -> bool {
    total > 0 && free == 0
}
