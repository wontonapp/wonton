//! 原来 Bot 钉在聊天页面板上的页面的记录（ADR-0052，已废弃）：每个插件一个 JSON 文件。
//!
//! 钉页能力已去掉，没有写入方；只剩 MCP Apps 宿主（`gateway::mcp_apps`）按钉页 ID 读。收件箱（ADR-0119）
//! 接手宿主、改成按它自己存的页面取之后，这个模块和宿主里按钉页 ID 取页的路径一起删。

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use wonton_protocol::TabRef;

use super::manifest::PluginId;
use super::store::{StoreError, read_json};

/// 钉在哪：主聊天页，或某个房间。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PinScope {
    Main,
    Room { room_id: String },
}

/// 一个 Bot 钉的页面。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PanelPin {
    pub id: String,
    pub scope: PinScope,
    pub tab: TabRef,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct BotPins {
    #[serde(default)]
    pins: Vec<PanelPin>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct PluginPins {
    #[serde(default)]
    bots: BTreeMap<String, BotPins>,
}

/// 钉页记录，只读。
pub struct PinStore {
    dir: PathBuf,
}

impl PinStore {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// 按钉页标识找一个：返回插件、Bot 和那一项。
    pub fn find(&self, pin: &str) -> Result<Option<(PluginId, String, PanelPin)>, StoreError> {
        let entries = match std::fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(source) => {
                return Err(StoreError::Io {
                    path: self.dir.clone(),
                    source,
                });
            }
        };
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
            for (bot, entry) in pins.bots {
                if let Some(found) = entry.pins.into_iter().find(|p| p.id == pin) {
                    return Ok(Some((id, bot, found)));
                }
            }
        }
        Ok(None)
    }
}
