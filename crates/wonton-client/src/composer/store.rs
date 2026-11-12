//! 附件区存盘：状态一个 JSON，照片的原图和缩略图各一个文件。App 被杀后附件区和待发消息都还在，接着传、接着发。

use std::io;
use std::path::{Path, PathBuf};

use wonton_l10n::l10n;
use wonton_protocol::CoreError;

use super::state::State;
use crate::home::write_private;
use crate::l10n::{CATALOG, language};

const STATE: &str = "composer.json";
const THUMBNAIL: &str = "thumb";

pub(crate) struct Disk {
    dir: PathBuf,
}

fn storage_error(path: &Path, error: &io::Error) -> CoreError {
    CoreError::Storage {
        reason: l10n!(
            CATALOG,
            language(),
            "composer-store-failed",
            path = path.display().to_string(),
            reason = error.to_string()
        ),
    }
}

impl Disk {
    pub(crate) fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// 读回上次的状态。没有文件时是空的；读不懂时记一条日志、从空的开始：附件区丢了主人可以再选，App 不能因此打不开。
    pub(crate) fn load(&self) -> State {
        let path = self.dir.join(STATE);
        match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|error| {
                tracing::warn!(%error, "the composer's saved state is unreadable, starting empty");
                State::default()
            }),
            Err(error) if error.kind() == io::ErrorKind::NotFound => State::default(),
            Err(error) => {
                tracing::warn!(%error, "could not read the composer's saved state, starting empty");
                State::default()
            }
        }
    }

    pub(crate) fn save(&self, state: &State) -> Result<(), CoreError> {
        let path = self.dir.join(STATE);
        std::fs::create_dir_all(&self.dir).map_err(|error| storage_error(&self.dir, &error))?;
        let bytes = serde_json::to_vec(state).expect("the composer state serializes");
        write_private(&path, &bytes).map_err(|error| storage_error(&path, &error))
    }

    fn image_path(&self, id: &str) -> PathBuf {
        self.dir.join(id)
    }

    fn thumbnail_path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.{THUMBNAIL}"))
    }

    pub(crate) fn save_image(
        &self,
        id: &str,
        data: &[u8],
        thumbnail: Option<&[u8]>,
    ) -> Result<(), CoreError> {
        std::fs::create_dir_all(&self.dir).map_err(|error| storage_error(&self.dir, &error))?;
        let path = self.image_path(id);
        write_private(&path, data).map_err(|error| storage_error(&path, &error))?;
        if let Some(thumbnail) = thumbnail {
            let path = self.thumbnail_path(id);
            write_private(&path, thumbnail).map_err(|error| storage_error(&path, &error))?;
        }
        Ok(())
    }

    pub(crate) fn image(&self, id: &str) -> Result<Vec<u8>, CoreError> {
        let path = self.image_path(id);
        std::fs::read(&path).map_err(|error| storage_error(&path, &error))
    }

    pub(crate) fn thumbnail(&self, id: &str) -> Result<Option<Vec<u8>>, CoreError> {
        let path = self.thumbnail_path(id);
        match std::fs::read(&path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(storage_error(&path, &error)),
        }
    }

    /// 附件不要了或者发出去了。删不掉只记日志：留下的是本机的几个文件，不影响发送。
    pub(crate) fn forget(&self, id: &str) {
        for path in [self.image_path(id), self.thumbnail_path(id)] {
            match std::fs::remove_file(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => tracing::info!(%error, "could not delete a composer image"),
            }
        }
    }
}
