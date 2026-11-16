//! Passwords used by third-party HTTP clients (ADR-0021).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tokio::sync::watch;

use crate::secret::constant_time_eq;

/// 存放位置（相对核心数据目录）：0600 的 JSON，明文值主人能在 App 里看（ADR-0021「在 App 里」）。
pub(crate) const FILE_NAME: &str = "clients/passwords.json";

const TOKEN_ALPHABET: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Record {
    id: String,
    label: String,
    value: String,
    /// 建它的设备名，App 里显示用；不知道时为空。
    #[serde(default)]
    created_by_device: Option<String>,
    /// 建它的设备 ID；设备吊销按它连带（ADR-0046）。
    #[serde(default)]
    created_by: Option<String>,
    /// 看过它的设备 ID → 最近一次取值时间（Unix 毫秒）。
    #[serde(default)]
    viewed_by: BTreeMap<String, u64>,
    #[serde(default)]
    pending: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct File {
    #[serde(default)]
    plugins: BTreeMap<String, Vec<Record>>,
}

#[derive(Debug, thiserror::Error)]
pub enum PasswordError {
    #[error("read or write client password storage failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("client password storage is corrupt: {0}")]
    Corrupt(String),
    #[error("client password was not found")]
    NotFound,
    #[error("client password label is invalid")]
    InvalidLabel,
}

/// （插件，密码 ID）→ 吊销信号：吊销时置位，转发层据此断开已升级的连接（ADR-0021「吊销」）。
type Guards = Arc<Mutex<BTreeMap<(String, String), watch::Sender<bool>>>>;

#[derive(Clone)]
pub struct PasswordStore {
    path: PathBuf,
    state: Arc<Mutex<File>>,
    guards: Guards,
}

#[derive(Debug, Clone)]
pub struct Created {
    pub id: String,
    pub value: String,
}

impl PasswordStore {
    pub fn open(data_dir: impl AsRef<Path>) -> Result<Self, PasswordError> {
        let path = data_dir.as_ref().join(FILE_NAME);
        let state = match fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text)
                .map_err(|error| PasswordError::Corrupt(error.to_string()))?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => File::default(),
            Err(error) => return Err(error.into()),
        };
        let store = Self {
            path,
            state: Arc::new(Mutex::new(state)),
            guards: Arc::new(Mutex::new(BTreeMap::new())),
        };
        store.ensure_mode()?;
        Ok(store)
    }

    /// `device` 是（设备 ID，设备名）：设备吊销按 ID 连带（ADR-0046），界面显示用名字。
    pub fn create(
        &self,
        plugin: &str,
        label: &str,
        device: Option<(&str, Option<&str>)>,
    ) -> Result<Created, PasswordError> {
        if label.trim().is_empty() || label.len() > 128 || label.contains(['\n', '\r']) {
            return Err(PasswordError::InvalidLabel);
        }
        let mut state = self.state.lock().expect("client password lock poisoned");
        let records = state.plugins.entry(plugin.to_owned()).or_default();
        let id = next_id(records);
        let value = random_base32();
        records.push(Record {
            id: id.clone(),
            label: label.to_owned(),
            value: value.clone(),
            created_by_device: device.and_then(|(_, name)| name).map(str::to_owned),
            created_by: device.map(|(id, _)| id.to_owned()),
            viewed_by: BTreeMap::new(),
            pending: false,
        });
        self.save_locked(&state)?;
        self.guards
            .lock()
            .expect("client password guard lock poisoned")
            .insert((plugin.to_owned(), id.clone()), watch::channel(false).0);
        Ok(Created { id, value })
    }

    pub fn read(
        &self,
        plugin: &str,
        id: &str,
        device: Option<&str>,
    ) -> Result<String, PasswordError> {
        let mut state = self.state.lock().expect("client password lock poisoned");
        let record = state
            .plugins
            .get_mut(plugin)
            .and_then(|records| records.iter_mut().find(|record| record.id == id))
            .ok_or(PasswordError::NotFound)?;
        record
            .viewed_by
            .insert(device.unwrap_or("owner_token").to_owned(), now_ms());
        let value = record.value.clone();
        self.save_locked(&state)?;
        Ok(value)
    }

    pub fn authenticate(
        &self,
        plugin: &str,
        supplied: &str,
    ) -> Result<Option<watch::Receiver<bool>>, PasswordError> {
        let state = self.state.lock().expect("client password lock poisoned");
        let Some(records) = state.plugins.get(plugin) else {
            return Ok(None);
        };
        let Some(record) = records
            .iter()
            .find(|record| !record.pending && constant_time_eq(&record.value, supplied))
        else {
            return Ok(None);
        };
        let key = (plugin.to_owned(), record.id.clone());
        let mut guards = self
            .guards
            .lock()
            .expect("client password guard lock poisoned");
        Ok(Some(
            guards
                .entry(key)
                .or_insert_with(|| watch::channel(false).0)
                .subscribe(),
        ))
    }

    pub fn revoke(&self, plugin: &str, id: &str) -> Result<(), PasswordError> {
        let mut state = self.state.lock().expect("client password lock poisoned");
        {
            let records = state
                .plugins
                .get_mut(plugin)
                .ok_or(PasswordError::NotFound)?;
            let Some(record) = records.iter_mut().find(|record| record.id == id) else {
                return Err(PasswordError::NotFound);
            };
            record.pending = true;
        }
        self.save_locked(&state)?;
        if let Some(sender) = self
            .guards
            .lock()
            .expect("client password guard lock poisoned")
            .get(&(plugin.to_owned(), id.to_owned()))
        {
            sender.send_replace(true);
        }
        state
            .plugins
            .get_mut(plugin)
            .expect("password record disappeared")
            .retain(|record| record.id != id);
        self.save_locked(&state)
    }

    pub fn revoke_for_device(&self, device: &str) -> Result<(), PasswordError> {
        let mut state = self.state.lock().expect("client password lock poisoned");
        let mut revoked = Vec::new();
        for (plugin, records) in &mut state.plugins {
            for record in records.iter_mut() {
                if record.created_by.as_deref() == Some(device)
                    || record.viewed_by.contains_key(device)
                {
                    record.pending = true;
                    revoked.push((plugin.clone(), record.id.clone()));
                }
            }
        }
        self.save_locked(&state)?;
        let guards = self
            .guards
            .lock()
            .expect("client password guard lock poisoned");
        for key in &revoked {
            if let Some(sender) = guards.get(key) {
                sender.send_replace(true);
            }
        }
        for (plugin, id) in revoked {
            if let Some(records) = state.plugins.get_mut(&plugin) {
                records.retain(|record| record.id != id);
            }
        }
        self.save_locked(&state)
    }

    pub fn remove_plugin(&self, plugin: &str) -> Result<(), PasswordError> {
        let mut state = self.state.lock().expect("client password lock poisoned");
        if let Some(records) = state.plugins.remove(plugin) {
            let guards = self
                .guards
                .lock()
                .expect("client password guard lock poisoned");
            for record in records {
                if let Some(sender) = guards.get(&(plugin.to_owned(), record.id)) {
                    sender.send_replace(true);
                }
            }
        }
        self.save_locked(&state)
    }

    fn save_locked(&self, state: &File) -> Result<(), PasswordError> {
        let parent = self
            .path
            .parent()
            .ok_or_else(|| PasswordError::Corrupt("password path has no parent".to_owned()))?;
        fs::create_dir_all(parent)?;
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
        let tmp = self.path.with_extension("json.tmp");
        let text = serde_json::to_string_pretty(state)
            .map_err(|error| PasswordError::Corrupt(error.to_string()))?;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)?;
        std::io::Write::write_all(&mut file, text.as_bytes())?;
        file.sync_all()?;
        fs::rename(tmp, &self.path)?;
        self.ensure_mode()
    }

    fn ensure_mode(&self) -> Result<(), PasswordError> {
        if self.path.exists() {
            fs::set_permissions(&self.path, fs::Permissions::from_mode(0o600))?;
        }
        Ok(())
    }
}

fn next_id(records: &[Record]) -> String {
    let mut number = records.len() + 1;
    loop {
        let id = format!("pw-{number}");
        if !records.iter().any(|record| record.id == id) {
            return id;
        }
        number += 1;
    }
}

fn random_base32() -> String {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).expect("OS randomness unavailable");
    let mut output = String::with_capacity(26);
    let mut buffer = 0u32;
    let mut bits = 0u8;
    for byte in bytes {
        buffer = (buffer << 8) | u32::from(byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            output.push(TOKEN_ALPHABET[((buffer >> bits) & 31) as usize] as char);
        }
    }
    if bits != 0 {
        output.push(TOKEN_ALPHABET[((buffer << (5 - bits)) & 31) as usize] as char);
    }
    output
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis() as u64)
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    #[test]
    fn stored_0600() {
        let dir = tempfile::tempdir().unwrap();
        let store = PasswordStore::open(dir.path()).unwrap();
        let created = store
            .create("app.demo", "phone", Some(("device-a", Some("iPad"))))
            .unwrap();
        assert_eq!(created.value.len(), 26);
        assert!(
            created
                .value
                .bytes()
                .all(|byte| TOKEN_ALPHABET.contains(&byte))
        );
        assert_eq!(
            fs::metadata(dir.path().join(FILE_NAME))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }

    #[test]
    fn revocation_invalidates_authentication_and_stream_guard() {
        let dir = tempfile::tempdir().unwrap();
        let store = PasswordStore::open(dir.path()).unwrap();
        let created = store.create("app.demo", "phone", None).unwrap();
        let receiver = store
            .authenticate("app.demo", &created.value)
            .unwrap()
            .unwrap();
        store.revoke("app.demo", "pw-1").unwrap();
        assert!(*receiver.borrow());
        assert!(
            store
                .authenticate("app.demo", &created.value)
                .unwrap()
                .is_none()
        );
    }
}
