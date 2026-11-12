use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use tokio::sync::watch;
use wonton_l10n::l10n;
use wonton_matrix::MatrixSession;
use wonton_protocol::{CoreError, MediaHandle};

use super::view::core_error;
use crate::l10n::{CATALOG, language};

type Download = watch::Receiver<Option<Result<String, CoreError>>>;

pub(super) struct AvatarFiles {
    inner: Arc<Inner>,
}

struct Inner {
    directory: PathBuf,
    pending: Mutex<HashMap<String, Download>>,
}

impl AvatarFiles {
    pub(super) fn new(data_dir: &Path) -> Self {
        Self {
            inner: Arc::new(Inner {
                directory: cache_directory(data_dir),
                pending: Mutex::new(HashMap::new()),
            }),
        }
    }

    pub(super) async fn get(
        &self,
        session: Option<Arc<MatrixSession>>,
        media: MediaHandle,
    ) -> Result<String, CoreError> {
        let mxc = plain_mxc(&media.token)?;
        let path = self.inner.directory.join(cache_key(&mxc));
        let mut download = {
            let mut pending = self
                .inner
                .pending
                .lock()
                .expect("avatar download lock poisoned");
            if let Some(download) = pending.get(&mxc) {
                download.clone()
            } else {
                let (result, download) = watch::channel(None);
                pending.insert(mxc.clone(), download.clone());
                let inner = self.inner.clone();
                // The job owns publication and cleanup, so cancelling any waiter cannot leave
                // a partial file or release the single-flight slot during a pending download.
                tokio::spawn(async move {
                    let outcome = download_file(session, media, path, mxc.clone()).await;
                    result.send_replace(Some(outcome));
                    inner
                        .pending
                        .lock()
                        .expect("avatar download lock poisoned")
                        .remove(&mxc);
                });
                download
            }
        };
        loop {
            if let Some(result) = download.borrow_and_update().clone() {
                return result;
            }
            download.changed().await.map_err(storage_error)?;
        }
    }
}

fn plain_mxc(token: &str) -> Result<String, CoreError> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Plain {
        url: String,
    }
    let Plain { url: mxc } = serde_json::from_str(token).map_err(|_| CoreError::InvalidInput {
        reason: "Avatar media handle must contain an unencrypted MXC URL".to_owned(),
    })?;
    // Keep MXC validation at the Matrix facade; encrypted MediaSource objects never reach it.
    wonton_matrix::media_token(&mxc).map_err(|error| core_error(&error))?;
    Ok(mxc)
}

// FNV-1a is specified here rather than relying on std's version-dependent hasher.
// It only chooses a directory: cached_file must compare the full MXC before serving bytes.
fn cache_key(mxc: &str) -> String {
    let hash = mxc
        .as_bytes()
        .iter()
        .fold(0xcbf29ce484222325_u64, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
        });
    format!("{hash:016x}")
}

fn cache_directory(data_dir: &Path) -> PathBuf {
    let parent = data_dir.parent();
    if let Some(support) =
        parent.filter(|path| path.file_name().is_some_and(|n| n == "Application Support"))
        && let (Some(library), Some(app)) = (support.parent(), data_dir.file_name())
    {
        return library.join("Caches").join(app).join("avatars");
    }
    if data_dir.file_name().is_some_and(|name| name == "files")
        && let Some(app) = parent
    {
        return app.join("cache").join("avatars");
    }
    data_dir.join("cache").join("avatars")
}

async fn download_file(
    session: Option<Arc<MatrixSession>>,
    media: MediaHandle,
    path: PathBuf,
    mxc: String,
) -> Result<String, CoreError> {
    let check_path = path.clone();
    let check_mxc = mxc.clone();
    if let Some(cached) = tokio::task::spawn_blocking(move || cached_file(&check_path, &check_mxc))
        .await
        .map_err(storage_error)??
    {
        return Ok(cached);
    }
    let session = session.ok_or_else(|| CoreError::NotReady {
        reason: "Matrix is not signed in and the avatar is not cached".to_owned(),
    })?;
    let bytes = session
        .download_media(&media.token)
        .await
        .map_err(|error| core_error(&error))?;
    // Blocking publication runs to completion even if its async caller is dropped; cleanup
    // never races a still-running async write. Media and identity are committed together.
    tokio::task::spawn_blocking(move || publish(&path, &mxc, &bytes))
        .await
        .map_err(storage_error)?
}

fn cached_file(path: &Path, mxc: &str) -> Result<Option<String>, CoreError> {
    match std::fs::read_to_string(path.join("mxc")) {
        Ok(stored) if stored == mxc => {
            let media = path.join("media");
            if !std::fs::metadata(&media).map_err(storage_error)?.is_file() {
                return Err(storage_error(l10n!(
                    CATALOG,
                    language(),
                    "chat-avatar-cache-not-file"
                )));
            }
            path_string(&media).map(Some)
        }
        Ok(_) => Err(storage_error(l10n!(
            CATALOG,
            language(),
            "chat-avatar-cache-hash-collision"
        ))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            match std::fs::symlink_metadata(path) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(error) => Err(storage_error(error)),
                Ok(_) => Err(storage_error(l10n!(
                    CATALOG,
                    language(),
                    "chat-avatar-cache-missing-mxc"
                ))),
            }
        }
        Err(error) => Err(storage_error(error)),
    }
}

fn publish(path: &Path, mxc: &str, bytes: &[u8]) -> Result<String, CoreError> {
    let directory = path
        .parent()
        .expect("the avatar cache path always has a parent");
    std::fs::create_dir_all(directory).map_err(storage_error)?;
    let mut random = [0; 16];
    getrandom::fill(&mut random).map_err(storage_error)?;
    let name: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
    let temporary = directory.join(format!(".{name}.part"));
    std::fs::create_dir(&temporary).map_err(storage_error)?;
    let _cleanup = PartialFile(temporary.clone());
    write_file(&temporary.join("mxc"), mxc.as_bytes())?;
    write_file(&temporary.join("media"), bytes)?;
    match std::fs::rename(&temporary, path) {
        Ok(()) => path_string(&path.join("media")),
        Err(error) => cached_file(path, mxc)?.ok_or_else(|| storage_error(error)),
    }
}

fn write_file(path: &Path, bytes: &[u8]) -> Result<(), CoreError> {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(storage_error)?;
    file.write_all(bytes).map_err(storage_error)?;
    file.sync_all().map_err(storage_error)
}

struct PartialFile(PathBuf);

impl Drop for PartialFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn path_string(path: &Path) -> Result<String, CoreError> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| CoreError::Storage {
            reason: l10n!(CATALOG, language(), "chat-avatar-cache-path-not-utf8"),
        })
}

fn storage_error(error: impl std::fmt::Display) -> CoreError {
    CoreError::Storage {
        reason: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cached_media_is_available_without_a_matrix_session() {
        let directory = tempfile::tempdir().unwrap();
        let files = AvatarFiles::new(directory.path());
        let mxc = "mxc://example.org/cached";
        let path = files.inner.directory.join(cache_key(mxc));
        let expected = publish(&path, mxc, b"cached avatar").unwrap();
        let media = MediaHandle {
            token: wonton_matrix::media_token(mxc).unwrap(),
        };
        assert_eq!(files.get(None, media).await.unwrap(), expected);
        let media = MediaHandle {
            token: wonton_matrix::media_token("mxc://example.org/missing").unwrap(),
        };
        assert!(matches!(
            files.get(None, media).await,
            Err(CoreError::NotReady { .. })
        ));
    }

    #[test]
    fn rejects_non_plain_or_invalid_sources() {
        for token in [
            r#"{"url":"mxc://example.org/media","key":{}}"#,
            r#"{"url":"https://example.org/media"}"#,
            r#"{"url":"mxc://example.org"}"#,
            r#"{"url":"mxc://"}"#,
            "null",
            "mxc://example.org/media",
        ] {
            assert!(matches!(
                plain_mxc(token),
                Err(CoreError::InvalidInput { .. })
            ));
        }
    }

    #[test]
    fn publication_is_complete_and_leaves_no_partial_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory
            .path()
            .join("avatars")
            .join(cache_key("mxc://example.org/media"));
        let mxc = "mxc://example.org/media";
        let media_path = path.join("media");
        assert_eq!(
            publish(&path, mxc, b"avatar").unwrap(),
            media_path.to_str().unwrap()
        );
        assert_eq!(std::fs::read(&media_path).unwrap(), b"avatar");
        assert_eq!(
            cached_file(&path, mxc).unwrap(),
            Some(media_path.to_str().unwrap().to_owned())
        );
        assert!(matches!(
            cached_file(&path, "mxc://other/media"),
            Err(CoreError::Storage { .. })
        ));
        assert_eq!(
            std::fs::read_dir(path.parent().unwrap()).unwrap().count(),
            1
        );
    }

    #[test]
    fn failed_publication_removes_its_partial_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("occupied");
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join("mxc"), "mxc://other/media").unwrap();
        assert!(matches!(
            publish(&path, "mxc://example.org/media", b"avatar"),
            Err(CoreError::Storage { .. })
        ));
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    }
}
