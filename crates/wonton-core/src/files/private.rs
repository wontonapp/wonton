//! Private upload storage uses the same no-follow filesystem operations as shares.
use super::record::Origin;
use super::share_fs::{Opened, ShareRef, Shares, TRANSFERS};
use super::{Failure, Files, share_failure};
use crate::plugin::{hostdir, languages::Languages};

impl Files {
    pub(crate) async fn remove_home(
        self: &std::sync::Arc<Self>,
        username: &str,
    ) -> Result<(), Failure> {
        let username = username.to_owned();
        self.blocking(move |files| {
            let _home = files
                .home_changes
                .write()
                .expect("home changes lock poisoned");
            crate::private_home::remove(&files.data_dir, &username).map_err(Failure::internal)?;
            files.registry.remove_home_uploads(&username)?;
            Ok(())
        })
        .await
    }

    pub(super) fn recover_homes(
        data_dir: &std::path::Path,
    ) -> Result<(), super::share_fs::ShareError> {
        let entries = match std::fs::read_dir(data_dir.join("core/home")) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(super::share_fs::ShareError::Io(e.to_string())),
        };
        for entry in entries {
            let entry = entry.map_err(|e| super::share_fs::ShareError::Io(e.to_string()))?;
            if !entry
                .file_type()
                .map_err(|error| super::share_fs::ShareError::Io(error.to_string()))?
                .is_dir()
            {
                return Err(super::share_fs::ShareError::Refused(
                    "Private home is not a real directory".into(),
                ));
            }
            let username = entry
                .file_name()
                .into_string()
                .map_err(|_| super::share_fs::ShareError::Refused("Invalid home name".into()))?;
            let path = crate::private_home::path(data_dir, &username)
                .map_err(super::share_fs::ShareError::Refused)?;
            if path.join(TRANSFERS).exists() {
                let shares = Shares::new(path);
                shares.prepare_removal()?;
                shares.recover_deleting()?;
            }
        }
        Ok(())
    }

    pub(super) fn home_shares(&self, username: &str, create: bool) -> Result<Shares, Failure> {
        let path = if create {
            crate::private_home::ensure(&self.data_dir, username, self.owner)
        } else {
            crate::private_home::path(&self.data_dir, username)
        }
        .map_err(Failure::internal)?;
        let shares = self.shares.at_root(path.clone());
        if create {
            hostdir::ensure_owned_dir(&path.join(TRANSFERS), self.owner)
                .map_err(|e| Failure::internal(e.to_string()))?;
            shares.prepare_removal()?;
        }
        Ok(shares)
    }

    pub(super) fn origin_shares(&self, origin: &Origin) -> Result<Shares, Failure> {
        match origin {
            Origin::Upload { username, .. } => self.home_shares(username, false),
            _ => Ok(self.shares.clone()),
        }
    }

    pub(super) async fn open_origin(
        &self,
        origin: &Origin,
        reference: ShareRef,
        langs: &Languages,
    ) -> Result<Opened, Failure> {
        let shares = self.origin_shares(origin)?;
        tokio::task::spawn_blocking(move || shares.open(&reference))
            .await
            .map_err(|e| Failure::internal(e.to_string()))?
            .map_err(|e| share_failure(e, langs))
    }
}
