//! 已配对设备的登记表。
//!
//! M0 还没有扫码配对（#16）：主人用令牌经管理 API 登记设备的 endpoint ID，所有设备都属于主人。登记表
//! 存在 `core/devices.json`，启动时读回，增删时整份重写。隧道入口和网关共用同一份白名单：吊销一台设备
//! 会立即断开它现有的隧道连接。

use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::sync::Mutex;

use iroh::EndpointId;
use serde::{Deserialize, Serialize};
use wonton_tunnel::Allowlist;

#[derive(Debug, thiserror::Error)]
pub enum DeviceError {
    #[error("读写设备登记表 {path} 失败：{source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("设备登记表 {path} 已损坏：{message}")]
    Corrupt { path: PathBuf, message: String },
}

#[derive(Serialize, Deserialize)]
struct Stored {
    devices: Vec<EndpointId>,
}

pub struct Devices {
    path: PathBuf,
    allowlist: Allowlist,
    /// 和白名单一致的有序副本，用来列出和持久化；锁同时串行化写文件。
    ids: Mutex<BTreeSet<EndpointId>>,
}

impl Devices {
    /// 读取登记表；文件不存在时从空表开始。
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, DeviceError> {
        let path = path.into();
        let ids: BTreeSet<EndpointId> = match fs::read_to_string(&path) {
            Ok(text) => {
                let stored: Stored =
                    serde_json::from_str(&text).map_err(|e| DeviceError::Corrupt {
                        path: path.clone(),
                        message: e.to_string(),
                    })?;
                stored.devices.into_iter().collect()
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => BTreeSet::new(),
            Err(source) => return Err(DeviceError::Io { path, source }),
        };
        Ok(Self {
            allowlist: Allowlist::new(ids.iter().copied()),
            ids: Mutex::new(ids),
            path,
        })
    }

    /// 隧道服务端用的白名单。
    pub fn allowlist(&self) -> &Allowlist {
        &self.allowlist
    }

    pub fn contains(&self, id: &EndpointId) -> bool {
        self.allowlist.contains(id)
    }

    pub fn list(&self) -> Vec<EndpointId> {
        self.lock().iter().copied().collect()
    }

    /// 登记一台设备。返回它之前是否不在表里。
    pub fn add(&self, id: EndpointId) -> Result<bool, DeviceError> {
        let mut ids = self.lock();
        if !ids.insert(id) {
            return Ok(false);
        }
        if let Err(e) = self.save(&ids) {
            ids.remove(&id);
            return Err(e);
        }
        self.allowlist.allow(id);
        Ok(true)
    }

    /// 吊销一台设备，并断开它现有的隧道连接。返回它之前是否在表里。
    pub fn remove(&self, id: &EndpointId) -> Result<bool, DeviceError> {
        let mut ids = self.lock();
        if !ids.remove(id) {
            return Ok(false);
        }
        // 先断开再写盘：写盘失败时设备也不能继续用，重启后会按旧表恢复，由调用方报错重试。
        self.allowlist.revoke(id);
        self.save(&ids)?;
        Ok(true)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeSet<EndpointId>> {
        self.ids.lock().expect("设备登记表锁中毒")
    }

    /// 先写临时文件再改名，避免写到一半断电留下损坏的登记表。
    fn save(&self, ids: &BTreeSet<EndpointId>) -> Result<(), DeviceError> {
        let io = |source| DeviceError::Io {
            path: self.path.clone(),
            source,
        };
        let stored = Stored {
            devices: ids.iter().copied().collect(),
        };
        let text = serde_json::to_string_pretty(&stored).map_err(|e| DeviceError::Corrupt {
            path: self.path.clone(),
            message: e.to_string(),
        })?;
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(io)?;
        }
        let tmp = self.path.with_extension("json.tmp");
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)
            .map_err(io)?;
        file.write_all(text.as_bytes()).map_err(io)?;
        file.sync_all().map_err(io)?;
        fs::rename(&tmp, &self.path).map_err(io)
    }
}

#[cfg(test)]
mod tests {
    use iroh::SecretKey;

    use super::*;

    #[test]
    fn devices_survive_restart_and_revocation_updates_the_allowlist() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("core/devices.json");
        let (phone, tablet) = (
            SecretKey::generate().public(),
            SecretKey::generate().public(),
        );
        let devices = Devices::open(&path).unwrap();
        assert!(devices.list().is_empty());
        assert!(devices.add(phone).unwrap());
        assert!(!devices.add(phone).unwrap());
        assert!(devices.add(tablet).unwrap());
        assert!(devices.allowlist().contains(&phone));

        let reopened = Devices::open(&path).unwrap();
        assert_eq!(reopened.list().len(), 2);
        assert!(reopened.contains(&tablet));
        assert!(reopened.remove(&phone).unwrap());
        assert!(!reopened.remove(&phone).unwrap());
        assert!(!reopened.allowlist().contains(&phone));
        assert_eq!(Devices::open(&path).unwrap().list(), vec![tablet]);
    }

    #[test]
    fn corrupt_registry_fails_instead_of_starting_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("devices.json");
        fs::write(&path, "{\"devices\": [\"not-an-id\"]}").unwrap();
        assert!(matches!(
            Devices::open(&path),
            Err(DeviceError::Corrupt { .. })
        ));
    }
}
