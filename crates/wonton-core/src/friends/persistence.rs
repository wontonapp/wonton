use std::fs;
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::Path;

use super::{FriendsError, Snapshot};

pub(super) fn read(path: &Path) -> Result<Snapshot, FriendsError> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Snapshot::default());
        }
        Err(source) => {
            return Err(FriendsError::Io {
                path: path.to_owned(),
                source,
            });
        }
    };
    let corrupt = |message: String| FriendsError::Corrupt {
        path: path.to_owned(),
        message,
    };
    let snapshot: Snapshot =
        serde_json::from_slice(&bytes).map_err(|error| corrupt(error.to_string()))?;
    snapshot
        .validate()
        .map_err(|error| corrupt(error.to_string()))?;
    Ok(snapshot)
}

/// Private temporary inode, file fsync, atomic rename, then directory fsync. As in plugin::store,
/// the rename is the commit point: a subsequent directory-sync error is only a durability warning,
/// not a failed transaction (reporting failure would leave published disk and memory divergent).
pub(super) fn write(path: &Path, snapshot: &Snapshot) -> Result<(), FriendsError> {
    let io = |source| FriendsError::Io {
        path: path.to_owned(),
        source,
    };
    let bytes = serde_json::to_vec_pretty(snapshot)
        .map_err(|error| FriendsError::Invalid(error.to_string()))?;
    let dir = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .map_err(io)?;
    let directory = fs::File::open(dir).map_err(io)?;
    let temp = dir.join(format!(".friends-{}.tmp", crate::secret::random_token()));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temp)
        .map_err(io)?;
    let result = (|| {
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::rename(&temp, path)?;
        use std::os::fd::AsFd;
        if let Err(error) = crate::plugin::store::sync_dir(directory.as_fd()) {
            tracing::warn!(path = %dir.display(), %error, "friend registry directory sync failed; a crash may lose the committed change");
        }
        Ok::<(), std::io::Error>(())
    })();
    if result.is_err() {
        // Never remove the destination on failure: it may still be the last complete registry.
        let _ = fs::remove_file(&temp);
    }
    result.map_err(io)
}
