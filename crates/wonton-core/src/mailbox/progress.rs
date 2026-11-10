//! SMTP 收下后才投递：记录已准入、已上传和已送达的人，重连不重复占频率或投递给已完成的人。

use std::collections::{BTreeMap, BTreeSet};

use std::fs;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::content::Body;

#[derive(Default, Deserialize, Serialize)]
pub(super) struct Progress {
    pub(super) decision: Option<Decision>,
    #[serde(default)]
    pub(super) uploaded: BTreeMap<String, Uploaded>,
    #[serde(default)]
    pub(super) delivered: BTreeSet<String>,
    /// 正文和要不要附原始 HTML：转换过一次就存下来，重扫和重启都不再算（转换失败也记）。
    #[serde(default)]
    pub(super) prepared: Option<Body>,
}

#[derive(Deserialize, Serialize)]
pub(super) enum Decision {
    Accepted {
        plugin_id: String,
        plugin_name: String,
    },
    Discard {
        hint: Option<String>,
    },
}

#[derive(Deserialize, Serialize)]
pub(super) struct Uploaded {
    pub(super) name: String,
    pub(super) content_type: String,
    pub(super) size: u64,
    pub(super) uri: String,
}

pub(super) struct Store(PathBuf);

impl Store {
    #[cfg(test)]
    pub(super) fn dir(&self) -> &Path {
        &self.0
    }
}

impl Store {
    pub(super) fn open(dir: &Path) -> anyhow::Result<Self> {
        fs::create_dir_all(dir)
            .with_context(|| format!("建立邮件投递记录目录 {}", dir.display()))?;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
        Ok(Self(dir.to_owned()))
    }

    fn path(&self, id: &str) -> PathBuf {
        self.0
            .join(format!("{:x}.json", Sha256::digest(id.as_bytes())))
    }

    pub(super) fn load(&self, id: &str) -> anyhow::Result<Progress> {
        let path = self.path(id);
        match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .with_context(|| format!("邮件投递记录 {} 已损坏", path.display())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Progress::default()),
            Err(error) => Err(error).with_context(|| format!("读取 {}", path.display())),
        }
    }

    pub(super) fn save(&self, id: &str, progress: &Progress) -> anyhow::Result<()> {
        let path = self.path(id);
        let temporary = path.with_extension("tmp");
        let mut file = fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(0o600)
            .open(&temporary)?;
        serde_json::to_writer(&mut file, progress)?;
        file.flush()?;
        file.sync_all()?;
        fs::rename(&temporary, &path)?;
        fs::File::open(&self.0)?.sync_all()?;
        Ok(())
    }

    pub(super) fn remove(&self, id: &str) -> anyhow::Result<()> {
        match fs::remove_file(self.path(id)) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
}
