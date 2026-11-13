use std::fs;
use std::io;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::{Path, PathBuf};

use crate::plugin::hostdir::{self, Owner};

/// 建不了或删不掉家目录。权限不足（EPERM/EACCES）单列：对外是“权限缺失”，不是笼统的存储错误。
#[derive(Debug, thiserror::Error)]
pub enum HomeError {
    /// 家目录的属主不是配置的 puid:pgid，当前进程改不动：改属主需要特权。正式安装的核心容器以 root 运行
    /// （deploy/install/compose.yaml 的 core 服务不设 user），有特权；开发时核心以自己的用户跑，目录已是自己
    /// 的属主，走不到这里。puid/pgid 来自 WONTON_PUID/WONTON_PGID（安装脚本写进安装目录的 .env，
    /// 默认 1000）。
    #[error("{path} 的属主要交给 {uid}:{gid}，当前进程没有权限：{source}")]
    Chown {
        path: PathBuf,
        uid: u32,
        gid: u32,
        source: io::Error,
    },
    /// 动目录内容（删除）被拒，前提同上。
    #[error("{path}：当前进程没有权限（正式安装的核心以 root 运行，有权限）：{source}")]
    Permission { path: PathBuf, source: io::Error },
    /// 用户名不合法（正常路径到不了：成员用户名在登记时已校验过）。
    #[error("{0}")]
    Invalid(String),
    #[error("{path}: {source}")]
    Io { path: PathBuf, source: io::Error },
    #[error("{path} 不是真实的目录")]
    NotDirectory { path: PathBuf },
}

pub fn path(data_dir: &Path, username: &str) -> Result<PathBuf, String> {
    crate::owner::check_username(username)?;
    Ok(data_dir.join("core").join("home").join(username))
}

pub fn ensure(data_dir: &Path, username: &str, owner: Owner) -> Result<PathBuf, HomeError> {
    let home = path(data_dir, username).map_err(HomeError::Invalid)?;
    let core = data_dir.join("core");
    for directory in [&core, &core.join("home"), &home] {
        let mut builder = fs::DirBuilder::new();
        builder.mode(0o700);
        match builder.create(directory) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => {
                return Err(HomeError::Io {
                    path: directory.to_path_buf(),
                    source: error,
                });
            }
        }
        let metadata = fs::symlink_metadata(directory).map_err(|error| HomeError::Io {
            path: directory.to_path_buf(),
            source: error,
        })?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(HomeError::NotDirectory {
                path: directory.to_path_buf(),
            });
        }
        // core 含别的核心记录，不能为挂载家目录改变它的属主或权限。
        if directory != &core {
            fs::set_permissions(directory, fs::Permissions::from_mode(0o700)).map_err(|error| {
                HomeError::Io {
                    path: directory.to_path_buf(),
                    source: error,
                }
            })?;
        }
    }
    hostdir::ensure_owned_dir(&home, owner).map_err(|error| match error {
        hostdir::DirError::Io { path, source }
            if source.kind() == io::ErrorKind::PermissionDenied =>
        {
            HomeError::Chown {
                path,
                uid: owner.uid,
                gid: owner.gid,
                source,
            }
        }
        hostdir::DirError::Symlink(path)
        | hostdir::DirError::NotDir(path)
        | hostdir::DirError::NotUnder { path, .. } => HomeError::NotDirectory { path },
        hostdir::DirError::Io { path, source } => HomeError::Io { path, source },
    })?;
    Ok(home)
}

pub fn remove(data_dir: &Path, username: &str) -> Result<(), HomeError> {
    let home = path(data_dir, username).map_err(HomeError::Invalid)?;
    for directory in [
        data_dir.join("core"),
        data_dir.join("core/home"),
        home.clone(),
    ] {
        let metadata = match fs::symlink_metadata(&directory) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => {
                return Err(HomeError::Io {
                    path: directory,
                    source: error,
                });
            }
        };
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(HomeError::NotDirectory { path: directory });
        }
    }
    fs::remove_dir_all(&home).map_err(|error| {
        if error.kind() == io::ErrorKind::PermissionDenied {
            HomeError::Permission {
                path: home.clone(),
                source: error,
            }
        } else {
            HomeError::Io {
                path: home.clone(),
                source: error,
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::MetadataExt;

    #[test]
    fn homes_are_private_and_removal_does_not_follow_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let metadata = fs::metadata(dir.path()).unwrap();
        let owner = Owner {
            uid: metadata.uid(),
            gid: metadata.gid(),
        };
        let alice = ensure(dir.path(), "alice", owner).unwrap();
        fs::write(alice.join("kept"), b"private").unwrap();
        assert_eq!(
            fs::metadata(&alice).unwrap().permissions().mode() & 0o777,
            0o700
        );
        let outside = dir.path().join("outside");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("kept"), b"other").unwrap();
        std::os::unix::fs::symlink(&outside, path(dir.path(), "bob").unwrap()).unwrap();
        assert!(ensure(dir.path(), "bob", owner).is_err());
        assert!(remove(dir.path(), "bob").is_err());
        assert_eq!(fs::read(outside.join("kept")).unwrap(), b"other");
        remove(dir.path(), "alice").unwrap();
        assert!(!alice.exists());
        remove(dir.path(), "alice").unwrap();
    }

    /// 锁住改属主的前提：正式安装的核心容器以 root 运行（core 服务不设 user），puid:pgid 的 chown 才有
    /// 特权。谁在这里加了 `user:`，[`HomeError::Chown`] 的前提就塌了。
    #[test]
    fn the_installed_core_runs_without_a_user_override_so_chown_has_privileges() {
        let mut in_core = false;
        let mut saw_core = false;
        for line in crate::install::COMPOSE.lines() {
            let is_service = line.len() > 3
                && line.starts_with("  ")
                && !line.starts_with("   ")
                && !line[2..].starts_with('#')
                && line.ends_with(':');
            if is_service {
                in_core = &line[2..line.len() - 1] == "core";
                saw_core |= in_core;
            }
            if in_core {
                assert!(
                    !line.starts_with("    user:"),
                    "core 服务不能设 user:，否则私人空间按 puid:pgid 改属主没有特权：{line}"
                );
            }
        }
        assert!(saw_core, "compose.yaml 里没有 core 服务");
    }
}
