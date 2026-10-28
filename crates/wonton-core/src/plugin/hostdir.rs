//! 宿主目录：核心替插件创建并交给 PUID:PGID 的目录（应用数据目录及其子目录、共享文件夹）。
//!
//! Docker 每次启动容器都会跟随软链接重新解析绑定挂载的源，chown 也会跟随软链接；插件可以在自己能写的
//! 目录里放软链接（上次安装留下的也算）。所以这里逐级用 lstat 检查，遇到软链接或不是目录就拒绝，
//! 绝不跟随。

use std::fs;
use std::io;
use std::os::unix::fs::{MetadataExt, lchown};
use std::path::{Component, Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum DirError {
    #[error("{} 是软链接：核心不跟随插件目录里的软链接", .0.display())]
    Symlink(PathBuf),
    #[error("{} 已存在但不是目录", .0.display())]
    NotDir(PathBuf),
    #[error("{} 不是 {} 下的规范路径", .path.display(), .root.display())]
    NotUnder { path: PathBuf, root: PathBuf },
    #[error("{}：{source}", .path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

/// 确保 `path` 是属于 uid:gid 的真实目录：不存在就创建（父目录必须已存在），是软链接或其他文件就拒绝。
/// 属主已经正确时不改，核心不以 root 运行时也能工作。
pub fn ensure_owned_dir(path: &Path, uid: u32, gid: u32) -> Result<(), DirError> {
    let io = |source| DirError::Io {
        path: path.to_owned(),
        source,
    };
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            fs::create_dir(path).map_err(io)?;
            fs::symlink_metadata(path).map_err(io)?
        }
        Err(e) => return Err(io(e)),
    };
    if meta.file_type().is_symlink() {
        return Err(DirError::Symlink(path.to_owned()));
    }
    if !meta.is_dir() {
        return Err(DirError::NotDir(path.to_owned()));
    }
    if meta.uid() == uid && meta.gid() == gid {
        return Ok(());
    }
    // 上面已确认不是软链接；lchown 保证即使这一刻被换掉也不会改到链接指向的目录。
    lchown(path, Some(uid), Some(gid)).map_err(io)
}

/// 从 `root` 开始逐级确保 `dirs` 里的每个目录（都在 `root` 下）：父目录先于子目录检查，
/// 所以路径上任何一级是软链接都会被拒绝，而不是被跟随。
pub fn prepare(root: &Path, dirs: &[PathBuf], uid: u32, gid: u32) -> Result<(), DirError> {
    let relative = dirs
        .iter()
        .map(|dir| {
            dir.strip_prefix(root)
                .ok()
                .filter(|rel| rel.components().all(|c| matches!(c, Component::Normal(_))))
                .ok_or_else(|| DirError::NotUnder {
                    path: dir.clone(),
                    root: root.to_owned(),
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    ensure_owned_dir(root, uid, gid)?;
    for rel in relative {
        let mut current = root.to_path_buf();
        for part in rel.components() {
            current.push(part);
            ensure_owned_dir(&current, uid, gid)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::symlink;

    use super::*;

    /// 测试目录和它的属主：新建的子目录继承同样的属主，不需要 chown 权限。
    fn root() -> (tempfile::TempDir, PathBuf, u32, u32) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("app");
        fs::create_dir(&root).unwrap();
        let meta = fs::metadata(&root).unwrap();
        (dir, root, meta.uid(), meta.gid())
    }

    #[test]
    fn nested_dirs_are_created_level_by_level() {
        let (_tmp, root, uid, gid) = root();
        let dirs = [root.join("a/b/c"), root.join("d")];
        prepare(&root, &dirs, uid, gid).unwrap();
        for dir in ["a", "a/b", "a/b/c", "d"] {
            assert!(
                fs::symlink_metadata(root.join(dir)).unwrap().is_dir(),
                "{dir}"
            );
        }
        // 再来一次（重装）不出错。
        prepare(&root, &dirs, uid, gid).unwrap();
    }

    #[test]
    fn a_symlink_at_any_level_is_refused_not_followed() {
        let (tmp, root, uid, gid) = root();
        let outside = tmp.path().join("outside");
        fs::create_dir(&outside).unwrap();
        // 上次安装的插件把可写目录下的 x 换成了指向外面的软链接。
        fs::create_dir(root.join("data")).unwrap();
        symlink(&outside, root.join("data/x")).unwrap();
        for dir in ["data/x", "data/x/y"] {
            let err = prepare(&root, &[root.join(dir)], uid, gid).unwrap_err();
            assert!(
                matches!(&err, DirError::Symlink(p) if p == &root.join("data/x")),
                "{dir}: {err}"
            );
        }
        assert!(
            fs::read_dir(&outside).unwrap().next().is_none(),
            "不能在软链接指向的目录里创建任何东西"
        );
        // 指向不存在路径的软链接也一样：不能替它创建目标。
        symlink(tmp.path().join("missing"), root.join("dangling")).unwrap();
        let err = prepare(&root, &[root.join("dangling")], uid, gid).unwrap_err();
        assert!(matches!(err, DirError::Symlink(_)), "{err}");
        assert!(!tmp.path().join("missing").exists());
    }

    #[test]
    fn a_symlinked_root_is_refused_and_its_target_keeps_its_owner() {
        let (tmp, root, uid, gid) = root();
        let link = tmp.path().join("link");
        symlink(&root, &link).unwrap();
        // 属主不同才会去 chown：跟随软链接的旧实现会改掉目标目录的属主（以 root 运行时）。
        let err = ensure_owned_dir(&link, uid + 1, gid + 1).unwrap_err();
        assert!(matches!(err, DirError::Symlink(_)), "{err}");
        let meta = fs::metadata(&root).unwrap();
        assert_eq!((meta.uid(), meta.gid()), (uid, gid));
        let err = prepare(&link, &[link.join("data")], uid, gid).unwrap_err();
        assert!(matches!(err, DirError::Symlink(_)), "{err}");
        assert!(!root.join("data").exists());
    }

    #[test]
    fn files_and_paths_outside_the_root_are_refused() {
        let (_tmp, root, uid, gid) = root();
        fs::write(root.join("file"), "").unwrap();
        let err = prepare(&root, &[root.join("file/sub")], uid, gid).unwrap_err();
        assert!(matches!(err, DirError::NotDir(_)), "{err}");
        for dir in [root.with_file_name("other"), root.join("a/../../other")] {
            let err = prepare(&root, std::slice::from_ref(&dir), uid, gid).unwrap_err();
            assert!(matches!(err, DirError::NotUnder { .. }), "{dir:?}: {err}");
        }
        assert!(!root.join("a").exists(), "路径不合法时什么都不创建");
    }
}
