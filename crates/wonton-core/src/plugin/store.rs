//! 已安装插件的记录：每个插件一个 JSON 文件，核心重启后据此恢复路由和 OIDC 客户端。
//!
//! 记录里有 OIDC 客户端密钥，文件权限设为 0600。

use std::fs;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::manifest::{Permission, PluginId};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledRecord {
    pub id: PluginId,
    pub version: String,
    /// 原始 compose 文本。
    pub source: String,
    pub granted_permissions: Vec<Permission>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oidc_client_secret: Option<String>,
    pub installed_at: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("读写插件记录 {path} 失败：{source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("插件记录 {path} 已损坏：{message}")]
    Corrupt { path: PathBuf, message: String },
}

pub struct Store {
    dir: PathBuf,
}

impl Store {
    pub fn open(dir: impl Into<PathBuf>) -> Result<Self, StoreError> {
        let dir = dir.into();
        fs::create_dir_all(&dir).map_err(|source| StoreError::Io {
            path: dir.clone(),
            source,
        })?;
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).map_err(|source| {
            StoreError::Io {
                path: dir.clone(),
                source,
            }
        })?;
        Ok(Self { dir })
    }

    fn path(&self, id: &PluginId) -> PathBuf {
        self.dir.join(format!("{id}.json"))
    }

    pub fn load_all(&self) -> Result<Vec<InstalledRecord>, StoreError> {
        let io = |path: &Path| {
            let path = path.to_path_buf();
            move |source| StoreError::Io { path, source }
        };
        let mut records = Vec::new();
        for entry in fs::read_dir(&self.dir).map_err(io(&self.dir))? {
            let path = entry.map_err(io(&self.dir))?.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let text = fs::read_to_string(&path).map_err(io(&path))?;
            let record = serde_json::from_str(&text).map_err(|e| StoreError::Corrupt {
                path: path.clone(),
                message: e.to_string(),
            })?;
            records.push(record);
        }
        records.sort_by(|a: &InstalledRecord, b| a.id.cmp(&b.id));
        Ok(records)
    }

    /// 先写临时文件再改名，避免写到一半断电留下损坏的记录。
    pub fn save(&self, record: &InstalledRecord) -> Result<(), StoreError> {
        let path = self.path(&record.id);
        let tmp = path.with_extension("json.tmp");
        let io = |source| StoreError::Io {
            path: path.clone(),
            source,
        };
        let text = serde_json::to_string_pretty(record).map_err(|e| StoreError::Corrupt {
            path: path.clone(),
            message: e.to_string(),
        })?;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)
            .map_err(io)?;
        file.write_all(text.as_bytes()).map_err(io)?;
        file.sync_all().map_err(io)?;
        fs::rename(&tmp, &path).map_err(io)
    }

    pub fn remove(&self, id: &PluginId) -> Result<(), StoreError> {
        let path = self.path(id);
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(source) => Err(StoreError::Io { path, source }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_survive_reopen_and_are_private() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("plugins")).unwrap();
        let record = InstalledRecord {
            id: PluginId::parse("app.test").unwrap(),
            version: "1.0.0".into(),
            source: "services: {}".into(),
            granted_permissions: vec!["storage.share.photos:read".parse().unwrap()],
            oidc_client_secret: Some("s3cret".into()),
            installed_at: 1,
        };
        store.save(&record).unwrap();
        let reopened = Store::open(dir.path().join("plugins")).unwrap();
        assert_eq!(reopened.load_all().unwrap(), vec![record.clone()]);
        let mode = fs::metadata(reopened.path(&record.id))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        reopened.remove(&record.id).unwrap();
        reopened.remove(&record.id).unwrap();
        assert!(reopened.load_all().unwrap().is_empty());
    }
}
