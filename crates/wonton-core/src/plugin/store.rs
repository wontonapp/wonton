//! 已安装插件的记录：每个插件一个 JSON 文件，核心重启后据此恢复路由和 OIDC 客户端。
//!
//! 记录里有整个插件包（compose 原文和它引用的文件）和 OIDC 客户端密钥，文件权限设为 0600。

use std::fs;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::manifest::{Permission, PluginId};
use super::package_files::PackageFiles;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledRecord {
    pub id: PluginId,
    pub version: String,
    /// 原始 compose 文本。
    pub source: String,
    /// 插件包里被引用的文件，供审计、恢复和升级比对。JSON 里是 base64。
    #[serde(
        default,
        skip_serializing_if = "PackageFiles::is_empty",
        with = "base64_files"
    )]
    pub files: PackageFiles,
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

/// 一个记录文件。ID 取自文件名，所以记录读不出或已损坏时也知道是哪个插件。
#[derive(Debug)]
pub struct RecordFile {
    pub id: PluginId,
    pub record: Result<InstalledRecord, StoreError>,
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

    /// 读出全部记录，按 ID 排序。单个记录读不出、已损坏或与文件名不符只体现在它自己的结果里；
    /// 只有记录目录本身读不了才返回错误。文件名不是插件 ID 的文件不是核心写的，记日志后跳过。
    pub fn load_all(&self) -> Result<Vec<RecordFile>, StoreError> {
        let io = |source| StoreError::Io {
            path: self.dir.clone(),
            source,
        };
        let mut files = Vec::new();
        for entry in fs::read_dir(&self.dir).map_err(io)? {
            let path = entry.map_err(io)?.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let id = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .and_then(|stem| PluginId::parse(stem).ok());
            let Some(id) = id else {
                tracing::warn!(path = %path.display(), "插件记录目录里有无法识别的文件，已跳过");
                continue;
            };
            let record = read_record(&path, &id);
            files.push(RecordFile { id, record });
        }
        files.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(files)
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

/// 文件内容在 JSON 里以 base64 保存。
mod base64_files {
    use std::collections::BTreeMap;

    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD;
    use serde::de::Error as _;
    use serde::{Deserialize, Deserializer, Serializer};

    use super::PackageFiles;

    pub fn serialize<S: Serializer>(
        files: &PackageFiles,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.collect_map(
            files
                .iter()
                .map(|(path, data)| (path, STANDARD.encode(data))),
        )
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<PackageFiles, D::Error> {
        BTreeMap::<String, String>::deserialize(deserializer)?
            .into_iter()
            .map(|(path, text)| {
                let data = STANDARD
                    .decode(&text)
                    .map_err(|e| D::Error::custom(format!("文件 {path} 不是合法的 base64：{e}")))?;
                Ok((path, data))
            })
            .collect()
    }
}

fn read_record(path: &Path, id: &PluginId) -> Result<InstalledRecord, StoreError> {
    let corrupt = |message: String| StoreError::Corrupt {
        path: path.to_owned(),
        message,
    };
    let text = fs::read_to_string(path).map_err(|source| StoreError::Io {
        path: path.to_owned(),
        source,
    })?;
    let record: InstalledRecord =
        serde_json::from_str(&text).map_err(|e| corrupt(e.to_string()))?;
    if &record.id != id {
        return Err(corrupt(format!(
            "记录里的 ID 是 {}，与文件名不符",
            record.id
        )));
    }
    Ok(record)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(id: &str) -> InstalledRecord {
        InstalledRecord {
            id: PluginId::parse(id).unwrap(),
            version: "1.0.0".into(),
            source: "services: {}".into(),
            files: [("config.yaml".to_owned(), b"a: \x01\xff".to_vec())].into(),
            granted_permissions: vec!["storage.share.photos:read".parse().unwrap()],
            oidc_client_secret: Some("s3cret".into()),
            installed_at: 1,
        }
    }

    fn readable(store: &Store) -> Vec<InstalledRecord> {
        let files = store.load_all().unwrap();
        files.into_iter().map(|f| f.record.unwrap()).collect()
    }

    #[test]
    fn records_survive_reopen_and_are_private() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("plugins")).unwrap();
        let record = record("app.test");
        store.save(&record).unwrap();
        let reopened = Store::open(dir.path().join("plugins")).unwrap();
        assert_eq!(readable(&reopened), vec![record.clone()]);
        let mode = fs::metadata(reopened.path(&record.id))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        reopened.remove(&record.id).unwrap();
        reopened.remove(&record.id).unwrap();
        assert!(reopened.load_all().unwrap().is_empty());
    }

    #[test]
    fn a_broken_record_is_reported_under_its_file_name_without_hiding_others() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let good = record("app.good");
        store.save(&good).unwrap();
        fs::write(dir.path().join("app.broken.json"), "{\"id\": ").unwrap();
        // 记录里的 ID 和文件名不符：按文件名报告，不能冒充另一个插件，否则卸载时删不掉这个文件。
        let other = serde_json::to_string(&record("app.other")).unwrap();
        fs::write(dir.path().join("app.renamed.json"), other).unwrap();
        // 文件名不是插件 ID：不是核心写的，跳过。
        fs::write(dir.path().join("notes.json"), "{}").unwrap();
        let files = store.load_all().unwrap();
        let ids: Vec<&str> = files.iter().map(|f| f.id.as_str()).collect();
        assert_eq!(ids, ["app.broken", "app.good", "app.renamed"]);
        assert!(matches!(files[0].record, Err(StoreError::Corrupt { .. })));
        assert_eq!(files[1].record.as_ref().unwrap(), &good);
        assert!(matches!(files[2].record, Err(StoreError::Corrupt { .. })));
    }
}
