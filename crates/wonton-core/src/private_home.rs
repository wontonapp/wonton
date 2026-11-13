use std::fs;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::{Path, PathBuf};

use crate::plugin::hostdir::{self, Owner};

pub fn path(data_dir: &Path, username: &str) -> Result<PathBuf, String> {
    crate::owner::check_username(username)?;
    Ok(data_dir.join("core").join("home").join(username))
}

pub fn ensure(data_dir: &Path, username: &str, owner: Owner) -> Result<PathBuf, String> {
    let home = path(data_dir, username)?;
    let core = data_dir.join("core");
    for directory in [&core, &core.join("home"), &home] {
        let mut builder = fs::DirBuilder::new();
        builder.mode(0o700);
        match builder.create(directory) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(format!("{}: {error}", directory.display())),
        }
        let metadata = fs::symlink_metadata(directory)
            .map_err(|error| format!("{}: {error}", directory.display()))?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(format!("{} is not a real directory", directory.display()));
        }
        // core 含别的核心记录，不能为挂载家目录改变它的属主或权限。
        if directory != &core {
            fs::set_permissions(directory, fs::Permissions::from_mode(0o700))
                .map_err(|error| format!("{}: {error}", directory.display()))?;
        }
    }
    hostdir::ensure_owned_dir(&home, owner).map_err(|error| error.to_string())?;
    Ok(home)
}

pub fn remove(data_dir: &Path, username: &str) -> Result<(), String> {
    let home = path(data_dir, username)?;
    for directory in [
        data_dir.join("core"),
        data_dir.join("core/home"),
        home.clone(),
    ] {
        let metadata = match fs::symlink_metadata(&directory) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(format!("{}: {error}", directory.display())),
        };
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(format!("{} is not a real directory", directory.display()));
        }
    }
    fs::remove_dir_all(&home).map_err(|error| format!("{}: {error}", home.display()))
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
}
