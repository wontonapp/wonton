//! 已配对设备的登记表。
//!
//! 设备扫码配对（`crate::pairing`）或由主人用令牌经管理 API 登记。设备归属人（ADR-0045）：主人的设备
//! 不写 `user`，家人登记在家人名下。登记表存在 `core/devices.json`，启动时读回，增删时整份重写。
//! 隧道入口和网关共用同一份白名单：吊销一台设备会立即断开它现有的隧道连接。

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

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

/// 一台已登记的设备。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Device {
    pub id: EndpointId,
    /// 配对时设备报的名字；主人经管理 API 登记的设备没有名字。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// 登记时间，Unix 秒。
    #[serde(default)]
    pub paired_at: u64,
    /// 设备归属的人（ADR-0045）：空是主人，家人是这个用户名。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct Stored {
    devices: Vec<StoredDevice>,
}

/// 扫码配对之前的登记表只有 endpoint ID，读的时候两种写法都认。
#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum StoredDevice {
    Full(Device),
    IdOnly(EndpointId),
}

pub struct Devices {
    path: PathBuf,
    allowlist: Allowlist,
    /// 和白名单一致的有序副本，用来列出和持久化；锁同时串行化写文件。
    devices: Mutex<BTreeMap<EndpointId, Device>>,
}

impl Devices {
    /// 读取登记表；文件不存在时从空表开始。
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, DeviceError> {
        let path = path.into();
        let devices: BTreeMap<EndpointId, Device> = match fs::read_to_string(&path) {
            Ok(text) => {
                let stored: Stored =
                    serde_json::from_str(&text).map_err(|e| DeviceError::Corrupt {
                        path: path.clone(),
                        message: e.to_string(),
                    })?;
                stored
                    .devices
                    .into_iter()
                    .map(|device| match device {
                        StoredDevice::Full(device) => (device.id, device),
                        StoredDevice::IdOnly(id) => (
                            id,
                            Device {
                                id,
                                name: None,
                                paired_at: 0,
                                user: None,
                            },
                        ),
                    })
                    .collect()
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(source) => return Err(DeviceError::Io { path, source }),
        };
        Ok(Self {
            allowlist: Allowlist::new(devices.keys().copied()),
            devices: Mutex::new(devices),
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

    /// 一台设备的记录；没有登记时为 `None`。
    pub fn get(&self, id: &EndpointId) -> Option<Device> {
        self.lock().get(id).cloned()
    }

    pub fn is_empty(&self) -> bool {
        self.lock().is_empty()
    }

    pub fn list(&self) -> Vec<Device> {
        self.lock().values().cloned().collect()
    }

    /// 归这个人的设备（主人传它的用户名，家人的设备 `user` 是家人）。
    pub fn of_user(&self, username: &str) -> Vec<Device> {
        self.lock()
            .values()
            .filter(|device| device.user.as_deref() == Some(username))
            .cloned()
            .collect()
    }

    /// 登记一台主人的设备。返回它之前是否不在表里；已经在表里时不改原来的记录。
    pub fn add(&self, id: EndpointId, name: Option<String>) -> Result<bool, DeviceError> {
        self.add_if(id, name, None, |_| true)
            .map(|added| added == Some(true))
    }

    /// `allowed` 看着登记前的表决定这台设备能不能登记，和登记在同一把锁里，两台设备同时来也只按一张表判断。
    /// 不允许时返回 `None`，否则同 [`add`](Self::add)；`user` 是归属人（空是主人）。
    pub fn add_if(
        &self,
        id: EndpointId,
        name: Option<String>,
        user: Option<String>,
        allowed: impl FnOnce(&BTreeMap<EndpointId, Device>) -> bool,
    ) -> Result<Option<bool>, DeviceError> {
        let mut devices = self.lock();
        if !allowed(&devices) {
            return Ok(None);
        }
        if devices.contains_key(&id) {
            return Ok(Some(false));
        }
        let paired_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_secs());
        devices.insert(
            id,
            Device {
                id,
                name,
                paired_at,
                user,
            },
        );
        if let Err(e) = self.save(&devices) {
            devices.remove(&id);
            return Err(e);
        }
        self.allowlist.allow(id);
        Ok(Some(true))
    }

    /// 吊销一台设备，并断开它现有的隧道连接。返回它之前是否在表里。
    pub fn remove(&self, id: &EndpointId) -> Result<bool, DeviceError> {
        let mut devices = self.lock();
        if devices.remove(id).is_none() {
            return Ok(false);
        }
        // 先断开再写盘：写盘失败时设备也不能继续用，重启后会按旧表恢复，由调用方报错重试。
        self.allowlist.revoke(id);
        self.save(&devices)?;
        Ok(true)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<EndpointId, Device>> {
        self.devices.lock().expect("设备登记表锁中毒")
    }

    /// 先写临时文件再改名，避免写到一半断电留下损坏的登记表。
    fn save(&self, devices: &BTreeMap<EndpointId, Device>) -> Result<(), DeviceError> {
        let io = |source| DeviceError::Io {
            path: self.path.clone(),
            source,
        };
        let stored = Stored {
            devices: devices.values().cloned().map(StoredDevice::Full).collect(),
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
        assert!(devices.is_empty());
        assert!(devices.add(phone, Some("手机".to_owned())).unwrap());
        assert!(!devices.add(phone, None).unwrap());
        assert!(devices.add(tablet, None).unwrap());
        assert!(devices.allowlist().contains(&phone));

        let reopened = Devices::open(&path).unwrap();
        assert_eq!(reopened.list().len(), 2);
        assert!(reopened.contains(&tablet));
        let named = reopened.list().into_iter().find(|d| d.id == phone).unwrap();
        assert_eq!(named.name.as_deref(), Some("手机"));
        assert!(named.paired_at > 0);
        assert!(reopened.remove(&phone).unwrap());
        assert!(!reopened.remove(&phone).unwrap());
        assert!(!reopened.allowlist().contains(&phone));
        let left: Vec<_> = Devices::open(&path)
            .unwrap()
            .list()
            .into_iter()
            .map(|d| d.id)
            .collect();
        assert_eq!(left, vec![tablet]);
    }

    #[test]
    fn a_registry_from_before_pairing_still_loads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("devices.json");
        let id = SecretKey::generate().public();
        fs::write(&path, format!("{{\"devices\": [\"{id}\"]}}")).unwrap();
        let devices = Devices::open(&path).unwrap();
        assert!(devices.contains(&id));
        assert_eq!(devices.list()[0].name, None);
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
