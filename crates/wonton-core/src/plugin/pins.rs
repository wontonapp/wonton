//! Bot 钉在聊天页面板上的页面（ADR-0052）：每个插件一个 JSON 文件，按 Bot 和面板位置存。
//!
//! Bot 不是靠写主人的账号数据钉页面，这份记录只存在服务器上。用户放的页面仍在账号数据里，
//! App 读出来和这里的合在一起显示。用户自己把 Bot 钉的页面拿掉、收回往主面板钉页的同意也写在这里。

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use wonton_protocol::{PanelScope, TabRef};

use super::manifest::PluginId;
use super::store::{StoreError, create_private_dir, read_json, remove_file, write_json};

/// 一个 Bot 钉的页面。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PanelPin {
    pub id: String,
    pub scope: PanelScope,
    pub tab: TabRef,
    pub title: String,
    #[serde(default)]
    pub attention: bool,
}

/// 一个 Bot 的记录：往主面板钉页的同意，和它钉的页面（按钉的先后）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BotPins {
    /// 用户同意过它往主面板钉页；第一次钉时问，拒绝或收回后为假。
    #[serde(default)]
    pub main_consent: bool,
    #[serde(default)]
    pub pins: Vec<PanelPin>,
}

/// 一个插件的全部 Bot。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginPins {
    #[serde(default)]
    pub bots: BTreeMap<String, BotPins>,
}

/// 钉页记录的存取。
pub struct PinStore {
    dir: PathBuf,
    /// 读改写串行：Bot 的工具和 App 的移除、收回可能同时改一份记录。
    lock: std::sync::Mutex<()>,
}

impl PinStore {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            lock: std::sync::Mutex::new(()),
        }
    }

    /// 读、改、写一份记录。
    pub fn with<T>(
        &self,
        id: &PluginId,
        change: impl FnOnce(&mut PluginPins) -> T,
    ) -> Result<T, StoreError> {
        let _guard = self.lock.lock().expect("钉页记录锁中毒");
        let mut pins = self.load(id)?;
        let value = change(&mut pins);
        self.save(id, &pins)?;
        Ok(value)
    }

    fn path(&self, id: &PluginId) -> PathBuf {
        self.dir.join(format!("{id}.json"))
    }

    pub fn load(&self, id: &PluginId) -> Result<PluginPins, StoreError> {
        Ok(read_json::<PluginPins>(&self.path(id))?.unwrap_or_default())
    }

    pub fn save(&self, id: &PluginId, pins: &PluginPins) -> Result<(), StoreError> {
        create_private_dir(&self.dir)?;
        write_json(&self.path(id), pins)
    }

    pub fn remove(&self, id: &PluginId) -> Result<(), StoreError> {
        remove_file(&self.path(id))
    }

    /// 所有插件的记录，按插件 ID 排序。
    pub fn all(&self) -> Result<BTreeMap<PluginId, PluginPins>, StoreError> {
        let entries = match std::fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
            Err(source) => {
                return Err(StoreError::Io {
                    path: self.dir.clone(),
                    source,
                });
            }
        };
        let mut all = BTreeMap::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let Some(id) = path
                .file_stem()
                .and_then(|s| s.to_str())
                .and_then(|s| PluginId::parse(s).ok())
            else {
                continue;
            };
            let pins: PluginPins = read_json(&path)?.unwrap_or_default();
            all.insert(id, pins);
        }
        Ok(all)
    }

    /// 按钉页标识找一个：返回插件、Bot 和那一项。
    pub fn find(&self, pin: &str) -> Result<Option<(PluginId, String, PanelPin)>, StoreError> {
        for (id, pins) in self.all()? {
            for (bot, entry) in pins.bots {
                if let Some(found) = entry.pins.into_iter().find(|p| p.id == pin) {
                    return Ok(Some((id, bot, found)));
                }
            }
        }
        Ok(None)
    }

    /// 插件卸载：它的页面一起拿掉。保留数据的卸载留着同意，重装后不再问；删数据的连记录一起删。
    pub fn forget_plugin(&self, id: &PluginId, purge: bool) -> Result<(), StoreError> {
        let _guard = self.lock.lock().expect("钉页记录锁中毒");
        if purge {
            return self.remove(id);
        }
        let mut pins = self.load(id)?;
        for entry in pins.bots.values_mut() {
            entry.pins.clear();
        }
        self.save(id, &pins)
    }
}
