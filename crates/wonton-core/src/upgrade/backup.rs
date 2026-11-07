//! 升级前的备份：`<数据目录>/core/upgrade/backup/<旧 digest>/`，目录 0700。每次都备份安装目录和核心数据；卷只在这一版
//! 声明会迁移它时备份。

use std::fs;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt, chown, symlink};
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};

use super::PROJECT;
use super::host::Mount;
use super::state::Layout;
use crate::l10n;
use crate::plugin::languages::Languages;

/// 备份写完才有这个文件：没写完的不能用来恢复。
const COMPLETE: &str = ".complete";
/// 这份备份可能是退回迁移前状态的唯一办法：只有之后某次 `completed` 才能删。记在备份目录里，不靠状态文件：状态文件
/// 下次升级就被覆盖了，`install.sh` 也会把它改名留档。
const KEEP: &str = ".keep";

pub const INSTALL: &str = "install";
pub const CORE: &str = "core";
pub const VOLUMES: &str = "volumes";

/// 核心数据里不备份、恢复时也不碰的目录：升级自己的状态，和没传完的上传。
fn core_skips(layout: &Layout) -> Vec<PathBuf> {
    let core = layout.core_dir();
    vec![core.join("upgrade"), core.join("files").join("uploads")]
}

/// `expired` 返回真时停下，返回错误。
pub type Expired<'a> = &'a dyn Fn() -> bool;

/// 目录里普通文件的总字节数。
pub fn tree_size(dir: &Path, skip: &[PathBuf]) -> anyhow::Result<u64> {
    let mut total = 0;
    for entry in fs::read_dir(dir).with_context(|| format!("读取 {}", dir.display()))? {
        let path = entry?.path();
        if skip.contains(&path) {
            continue;
        }
        let meta = fs::symlink_metadata(&path)?;
        total += if meta.is_dir() {
            tree_size(&path, skip)?
        } else {
            meta.len()
        };
    }
    Ok(total)
}

fn copy_entry(from: &Path, to: &Path) -> anyhow::Result<()> {
    // 文件操作的上下文是原始值（和系统报错一样），不进 Fluent；句子在上面的层写。
    let meta = fs::symlink_metadata(from).with_context(|| format!("read {}", from.display()))?;
    let kind = meta.file_type();
    if kind.is_symlink() {
        symlink(fs::read_link(from)?, to)?;
    } else if kind.is_file() {
        fs::copy(from, to).with_context(|| format!("copy {}", from.display()))?;
        let times = fs::FileTimes::new().set_modified(meta.modified()?);
        fs::File::options().write(true).open(to)?.set_times(times)?;
    } else {
        // 套接字和管道是进程运行时才有的，恢复了也没用。
        return Ok(());
    }
    chown(to, Some(meta.uid()), Some(meta.gid()))
        .or_else(|e| if kind.is_symlink() { Ok(()) } else { Err(e) })
        .with_context(|| format!("set the owner of {}", to.display()))?;
    if kind.is_file() {
        fs::set_permissions(to, fs::Permissions::from_mode(meta.mode()))?;
    }
    Ok(())
}

/// 把 `from` 的内容复制进 `to`，保留权限、属主和修改时间。`skip` 里的路径（完整路径）不复制。
pub fn copy_tree(from: &Path, to: &Path, skip: &[PathBuf], expired: Expired) -> anyhow::Result<()> {
    let meta = fs::metadata(from).with_context(|| format!("read {}", from.display()))?;
    fs::create_dir_all(to).with_context(|| format!("create {}", to.display()))?;
    for entry in fs::read_dir(from).with_context(|| format!("read {}", from.display()))? {
        if expired() {
            bail!("copying {} took too long", from.display());
        }
        let source = entry?.path();
        if skip.contains(&source) {
            continue;
        }
        let target = to.join(source.file_name().unwrap_or_default());
        if fs::symlink_metadata(&source)?.is_dir() {
            copy_tree(&source, &target, skip, expired)?;
        } else {
            copy_entry(&source, &target)?;
        }
    }
    chown(to, Some(meta.uid()), Some(meta.gid()))
        .with_context(|| format!("set the owner of {}", to.display()))?;
    fs::set_permissions(to, fs::Permissions::from_mode(meta.mode()))?;
    Ok(())
}

/// 清空目录的内容，`skip` 里的路径和它们的上级目录留着。目录本身可能是挂载点，不删。
fn clear_dir(dir: &Path, skip: &[PathBuf]) -> anyhow::Result<()> {
    for entry in fs::read_dir(dir).with_context(|| format!("read {}", dir.display()))? {
        let path = entry?.path();
        if skip.contains(&path) {
            continue;
        }
        let is_dir = fs::symlink_metadata(&path)?.is_dir();
        if is_dir && skip.iter().any(|s| s.starts_with(&path)) {
            clear_dir(&path, skip)?;
        } else if is_dir {
            fs::remove_dir_all(&path).with_context(|| format!("delete {}", path.display()))?;
        } else {
            fs::remove_file(&path).with_context(|| format!("delete {}", path.display()))?;
        }
    }
    Ok(())
}

/// 这次要备份的文件有多大（不含卷）。
pub fn files_size(layout: &Layout) -> anyhow::Result<u64> {
    let install = tree_size(&layout.install_dir, &[])?;
    Ok(install + tree_size(&layout.core_dir(), &core_skips(layout))?)
}

/// 建这次的备份目录。同一个 digest 留着的备份不能覆盖：那是退回迁移前状态的唯一办法。
pub fn create_dir(layout: &Layout, key: &str, id: &str) -> anyhow::Result<PathBuf> {
    let root = layout.backup_root();
    let mut dir = root.join(key);
    if dir.join(KEEP).exists() {
        dir = root.join(format!("{key}.{id}"));
    }
    if dir.exists() {
        fs::remove_dir_all(&dir).with_context(|| format!("删除 {}", dir.display()))?;
    }
    fs::create_dir_all(&root).with_context(|| format!("建立 {}", root.display()))?;
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&dir)
        .with_context(|| format!("建立 {}", dir.display()))?;
    Ok(dir)
}

/// 备份安装目录和核心数据。
pub fn copy_files(layout: &Layout, dir: &Path, expired: Expired) -> anyhow::Result<()> {
    copy_tree(&layout.install_dir, &dir.join(INSTALL), &[], expired)?;
    copy_tree(
        &layout.core_dir(),
        &dir.join(CORE),
        &core_skips(layout),
        expired,
    )
}

/// 记下备份写完了。写之前先把数据落盘：断电后有这个文件就说明备份是完整的。
pub fn mark_complete(dir: &Path) -> anyhow::Result<()> {
    let handle = fs::File::open(dir).with_context(|| format!("打开 {}", dir.display()))?;
    rustix::fs::syncfs(&handle).with_context(|| format!("同步 {}", dir.display()))?;
    super::state::write_atomic(&dir.join(COMPLETE), b"", 0o600)
}

/// 从备份恢复核心数据和安装目录。不碰 `upgrade/` 和 `files/uploads/`。
pub fn restore_files(layout: &Layout, dir: &Path, langs: &Languages) -> anyhow::Result<()> {
    if !dir.join(COMPLETE).exists() {
        bail!(
            "{}",
            l10n::text(
                langs,
                "upgrade-backup-incomplete",
                &[("path", dir.display().to_string().into())],
            )
        );
    }
    let never = || false;
    let skips = core_skips(layout);
    clear_dir(&layout.install_dir, &[])?;
    copy_tree(&dir.join(INSTALL), &layout.install_dir, &[], &never)?;
    clear_dir(&layout.core_dir(), &skips)?;
    copy_tree(&dir.join(CORE), &layout.core_dir(), &[], &never)
}

pub fn mark_keep(dir: &Path) -> anyhow::Result<()> {
    super::state::write_atomic(&dir.join(KEEP), b"", 0o600)
}

fn backups(layout: &Layout) -> anyhow::Result<Vec<PathBuf>> {
    let root = layout.backup_root();
    let entries = match fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e).with_context(|| format!("读取 {}", root.display())),
    };
    let mut dirs = Vec::new();
    for entry in entries {
        dirs.push(entry?.path());
    }
    Ok(dirs)
}

/// 这次升级之前可以删的旧备份：够不够放下这次的备份时，它们算作可用空间。
pub fn deletable(layout: &Layout) -> anyhow::Result<(Vec<PathBuf>, u64)> {
    let mut size = 0;
    let mut dirs = Vec::new();
    for dir in backups(layout)? {
        if !dir.join(KEEP).exists() {
            size += tree_size(&dir, &[])?;
            dirs.push(dir);
        }
    }
    Ok((dirs, size))
}

pub fn remove_all(dirs: &[PathBuf]) -> anyhow::Result<()> {
    for dir in dirs {
        fs::remove_dir_all(dir).with_context(|| format!("删除 {}", dir.display()))?;
    }
    Ok(())
}

/// `completed` 后只留这一次的，手动退回要用。之前留着的也不用了。
pub fn remove_others(layout: &Layout, keep: Option<&Path>) -> anyhow::Result<()> {
    let others: Vec<PathBuf> = backups(layout)?
        .into_iter()
        .filter(|dir| Some(dir.as_path()) != keep)
        .collect();
    remove_all(&others)
}

/// compose 项目里卷 `name` 在 Docker 里的名字。
pub fn compose_volume(name: &str) -> String {
    format!("{PROJECT}_{name}")
}

/// 把卷挂在 `/volume` 上的一次性容器用。
pub fn volume_mount(name: &str, read_only: bool) -> Mount {
    Mount::Volume {
        name: compose_volume(name),
        target: PathBuf::from("/volume"),
        read_only,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn langs() -> Languages {
        Languages::from_header(None)
    }

    struct Temp(PathBuf);

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn setup() -> (Temp, Layout) {
        let root =
            std::env::temp_dir().join(format!("wonton-backup-{}", crate::secret::random_token()));
        let layout = Layout {
            install_dir: root.join("opt"),
            data_dir: root.join("data"),
        };
        fs::create_dir_all(layout.install_dir.join("secrets")).unwrap();
        fs::create_dir_all(layout.core_dir().join("files/uploads")).unwrap();
        fs::create_dir_all(layout.core_dir().join("plugins")).unwrap();
        fs::create_dir_all(layout.upgrade_dir()).unwrap();
        fs::write(layout.env_path(), "old").unwrap();
        fs::write(layout.install_dir.join("secrets/key"), "secret").unwrap();
        fs::set_permissions(
            layout.install_dir.join("secrets/key"),
            fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        fs::write(layout.core_dir().join("devices.json"), "old").unwrap();
        fs::write(layout.core_dir().join("plugins/a.json"), "old").unwrap();
        fs::write(layout.core_dir().join("files/uploads/partial"), "up").unwrap();
        fs::write(layout.core_dir().join("files/kept"), "old").unwrap();
        fs::write(layout.state_path(), "state").unwrap();
        (Temp(root), layout)
    }

    fn back_up(layout: &Layout, key: &str, id: &str) -> PathBuf {
        let dir = create_dir(layout, key, id).unwrap();
        copy_files(layout, &dir, &|| false).unwrap();
        mark_complete(&dir).unwrap();
        dir
    }

    /// 恢复后就是升级前的样子，升级期间写的都没了；`upgrade/` 和没传完的上传不碰。
    #[test]
    fn restoring_brings_back_what_was_backed_up_and_leaves_the_rest() {
        let (_temp, layout) = setup();
        let dir = back_up(&layout, "sha256:old", "u1");
        assert!(!dir.join("core/upgrade").exists());
        assert!(!dir.join("core/files/uploads").exists());

        fs::write(layout.env_path(), "new").unwrap();
        fs::write(layout.install_dir.join("added-by-init"), "x").unwrap();
        fs::write(layout.core_dir().join("devices.json"), "new").unwrap();
        fs::write(layout.core_dir().join("paired-during-upgrade"), "x").unwrap();
        fs::remove_file(layout.core_dir().join("plugins/a.json")).unwrap();

        restore_files(&layout, &dir, &langs()).unwrap();

        assert_eq!(fs::read_to_string(layout.env_path()).unwrap(), "old");
        assert!(!layout.install_dir.join("added-by-init").exists());
        assert_eq!(
            fs::read_to_string(layout.core_dir().join("devices.json")).unwrap(),
            "old"
        );
        assert!(!layout.core_dir().join("paired-during-upgrade").exists());
        assert!(layout.core_dir().join("plugins/a.json").exists());
        assert_eq!(fs::read_to_string(layout.state_path()).unwrap(), "state");
        assert_eq!(
            fs::read_to_string(layout.core_dir().join("files/uploads/partial")).unwrap(),
            "up"
        );
        let key = fs::metadata(layout.install_dir.join("secrets/key")).unwrap();
        assert_eq!(key.mode() & 0o777, 0o600);
    }

    #[test]
    fn a_backup_that_was_not_finished_cannot_be_restored() {
        let (_temp, layout) = setup();
        let dir = create_dir(&layout, "sha256:old", "u1").unwrap();
        copy_files(&layout, &dir, &|| false).unwrap();
        assert!(restore_files(&layout, &dir, &langs()).is_err());
        assert_eq!(fs::read_to_string(layout.env_path()).unwrap(), "old");
    }

    #[test]
    fn the_backup_directory_is_private_and_copying_stops_at_the_limit() {
        let (_temp, layout) = setup();
        let dir = create_dir(&layout, "sha256:old", "u1").unwrap();
        assert_eq!(fs::metadata(&dir).unwrap().mode() & 0o777, 0o700);
        assert!(copy_files(&layout, &dir, &|| true).is_err());
    }

    /// 结果不确定的那份备份是退回迁移前状态的唯一办法：再升级时不删、不覆盖，只有之后某次 `completed` 才清掉。
    #[test]
    fn a_marked_backup_survives_later_upgrades_until_one_completes() {
        let (_temp, layout) = setup();
        let kept = back_up(&layout, "sha256:old", "u1");
        mark_keep(&kept).unwrap();
        let other = back_up(&layout, "sha256:other", "u2");

        let (dirs, size) = deletable(&layout).unwrap();
        assert_eq!(dirs, vec![other.clone()]);
        assert!(size > 0);

        fs::write(layout.core_dir().join("devices.json"), "later").unwrap();
        let again = back_up(&layout, "sha256:old", "u3");
        assert_ne!(again, kept);
        assert_eq!(
            fs::read_to_string(kept.join("core/devices.json")).unwrap(),
            "old"
        );

        remove_others(&layout, Some(&again)).unwrap();
        assert!(!kept.exists());
        assert!(!other.exists());
        assert!(again.exists());
    }
}
