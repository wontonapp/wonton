//! Bot 能往哪些路径存文件（ADR-0027）：记录目录下的 `save-grants/<插件 id>.json`。路径形如 `music/周杰伦`，按段比较。

use std::fs;
use std::io::ErrorKind;
use std::path::PathBuf;
use std::sync::Mutex;

use super::manager::PluginManager;
use super::manifest::PluginId;
use super::store::{StoreError, create_private_dir, read_json, write_json};

pub struct SaveGrantStore {
    dir: PathBuf,
    /// 改一条要先读再写。
    writing: Mutex<()>,
}

/// 授权 `granted` 包含不包含 `folder`。路径的段里没有 `/`，所以 `music/a` 不包含 `music/ab`。
pub fn covers(granted: &str, folder: &str) -> bool {
    folder == granted
        || folder
            .strip_prefix(granted)
            .is_some_and(|rest| rest.starts_with('/'))
}

impl SaveGrantStore {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            writing: Mutex::new(()),
        }
    }

    fn path(&self, id: &PluginId) -> PathBuf {
        self.dir.join(format!("{id}.json"))
    }

    pub fn list(&self, id: &PluginId) -> Result<Vec<String>, StoreError> {
        Ok(read_json(&self.path(id))?.unwrap_or_default())
    }

    /// 读不出旧的时报错，不覆盖：那些授权可能还在。被新授权包含的旧授权合并掉。
    pub fn add(&self, id: &PluginId, folder: &str) -> Result<(), StoreError> {
        let _writing = self.writing.lock().expect("授权表锁中毒");
        let mut grants = self.list(id)?;
        if grants.iter().any(|granted| covers(granted, folder)) {
            return Ok(());
        }
        grants.retain(|granted| !covers(folder, granted));
        grants.push(folder.to_owned());
        grants.sort();
        create_private_dir(&self.dir)?;
        write_json(&self.path(id), &grants)
    }

    pub fn remove(&self, id: &PluginId, folder: &str) -> Result<(), StoreError> {
        let _writing = self.writing.lock().expect("授权表锁中毒");
        let mut grants = self.list(id)?;
        grants.retain(|granted| granted != folder);
        write_json(&self.path(id), &grants)
    }

    pub fn clear(&self, id: &PluginId) -> Result<(), StoreError> {
        let _writing = self.writing.lock().expect("授权表锁中毒");
        let path = self.path(id);
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
            Err(source) => Err(StoreError::Io { path, source }),
        }
    }
}

impl PluginManager {
    /// 读不出来时当作没有授权，回到发卡片（ADR-0027）。
    pub fn save_grants(&self, id: &PluginId) -> Vec<String> {
        self.store.save_grants().list(id).unwrap_or_else(|error| {
            tracing::error!(plugin = %id, %error, "读存文件的授权失败，当作没有授权");
            Vec::new()
        })
    }

    pub fn add_save_grant(&self, id: &PluginId, folder: &str) -> Result<(), StoreError> {
        self.store.save_grants().add(id, folder)
    }

    pub fn remove_save_grant(&self, id: &PluginId, folder: &str) -> Result<(), StoreError> {
        self.store.save_grants().remove(id, folder)
    }

    /// 插件的应用数据目录（`WONTON_APP_DATA`）。
    pub fn app_data_dir(&self, id: &PluginId) -> PathBuf {
        self.cfg.apps_dir.join(id.slug())
    }
}

#[cfg(test)]
mod tests {
    use super::covers;

    #[test]
    fn a_grant_covers_the_folder_and_what_is_below_it() {
        assert!(covers("music", "music"));
        assert!(covers("music", "music/周杰伦/七里香"));
        assert!(!covers("music/a", "music/ab"));
        assert!(!covers("music/a", "music"));
    }
}
