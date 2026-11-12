use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
use std::os::fd::AsFd;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::Path;

use wonton_protocol::ShareRecipient;

use super::types::MAX_REVOCATIONS;
use super::{SharesError, Snapshot};

pub(super) fn read(path: &Path) -> Result<Snapshot, SharesError> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Snapshot::default());
        }
        Err(source) => {
            return Err(SharesError::Io {
                path: path.to_owned(),
                source,
            });
        }
    };
    let corrupt = |message| SharesError::Corrupt {
        path: path.to_owned(),
        message,
    };
    let snapshot: Snapshot =
        serde_json::from_slice(&bytes).map_err(|error| corrupt(error.to_string()))?;
    validate(&snapshot).map_err(corrupt)?;
    Ok(snapshot)
}

fn validate(snapshot: &Snapshot) -> Result<(), String> {
    let mut ids = BTreeSet::new();
    for stored in &snapshot.grants {
        let grant = &stored.grant;
        let counter = grant
            .id
            .strip_prefix("share-")
            .and_then(|value| value.parse::<u64>().ok());
        if !counter.is_some_and(|counter| {
            counter > 0 && counter <= snapshot.revision && grant.id == format!("share-{counter}")
        }) || !ids.insert(&grant.id)
            || grant.plugin_id.is_empty()
        {
            return Err("invalid or duplicate grant ID/plugin".into());
        }
        if stored.generation == 0
            || stored.generation > snapshot.revision
            || counter.is_some_and(|created| stored.generation < created)
        {
            return Err("invalid grant generation".into());
        }
        match &grant.recipient {
            ShareRecipient::Friend { user_id } => {
                ruma_common::UserId::parse(user_id).map_err(|e| e.to_string())?;
                if !stored.direct_generation.is_some_and(|generation| {
                    generation > 0 && generation <= stored.friend_revision
                }) {
                    return Err("invalid direct friendship generation".into());
                }
            }
            ShareRecipient::Group { group_id }
                if !group_id.is_empty() && stored.direct_generation.is_none() => {}
            _ => return Err("invalid grant recipient/binding".into()),
        }
    }
    if snapshot.revocations.len() > MAX_REVOCATIONS {
        return Err("too many retained revocations".into());
    }
    let mut revoked = BTreeSet::new();
    for revocation in &snapshot.revocations {
        ruma_common::UserId::parse(&revocation.user_id).map_err(|e| e.to_string())?;
        if revocation.plugin_id.is_empty()
            || !revoked.insert((&revocation.user_id, &revocation.plugin_id))
        {
            return Err("invalid or duplicate revocation".into());
        }
    }
    Ok(())
}

pub(super) fn write(path: &Path, snapshot: &Snapshot) -> Result<(), SharesError> {
    let io = |source| SharesError::Io {
        path: path.to_owned(),
        source,
    };
    let bytes = serde_json::to_vec_pretty(snapshot)
        .map_err(|error| SharesError::Invalid(error.to_string()))?;
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
    let temp = dir.join(format!(".shares-{}.tmp", crate::secret::random_token()));
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
        if let Err(error) = crate::plugin::store::sync_dir(directory.as_fd()) {
            tracing::warn!(path = %dir.display(), %error, "share registry directory sync failed; a crash may lose the committed change");
        }
        Ok::<(), std::io::Error>(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result.map_err(io)
}
