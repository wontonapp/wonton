//! `core/contacts.json`：每个本机 Bot 一张联系人表，按对方的 Matrix 用户 ID 一行。请求不过期，重启后还在。

use std::collections::BTreeMap;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use wonton_protocol::{BotRef, ContactEvent, ContactKind, ContactState};

/// 一行联系人。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Row {
    pub kind: ContactKind,
    pub state: ContactState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keeper_display_name: Option<String>,
    /// 跨机 Bot 请求的 ID：`Pending` 时是这边发出的，`Incoming` 时是对方发来的（回结果时带上）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    /// 对人：Bot 发出的那个私聊。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room_id: Option<String>,
    /// 最近一次变化的时刻（Unix 毫秒）；`Incoming` 时是收到请求的时刻。
    pub updated_at_ms: u64,
}

/// 一个 Bot 的联系人表。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BotRows {
    pub bot: Option<BotRef>,
    #[serde(default)]
    pub contacts: BTreeMap<String, Row>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stored {
    /// 键是 `<插件 id>/<Bot 名>`。
    #[serde(default)]
    pub bots: BTreeMap<String, BotRows>,
    /// 改了行、还没送到对方核心的事件，按产生的先后（[`super::outbox`]）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub outbox: Vec<Queued>,
}

/// 一条等着发给 `server` 的核心的事件。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Queued {
    pub server: String,
    pub event: ContactEvent,
}

pub fn key(bot: &BotRef) -> String {
    format!("{}/{}", bot.plugin_id, bot.bot)
}

impl Stored {
    pub fn rows(&self, bot: &BotRef) -> Option<&BTreeMap<String, Row>> {
        self.bots.get(&key(bot)).map(|rows| &rows.contacts)
    }

    pub fn row(&self, bot: &BotRef, user_id: &str) -> Option<&Row> {
        self.rows(bot)?.get(user_id)
    }

    pub fn put(&mut self, bot: &BotRef, user_id: &str, row: Row) {
        let entry = self.bots.entry(key(bot)).or_default();
        entry.bot = Some(bot.clone());
        entry.contacts.insert(user_id.to_owned(), row);
    }

    pub fn take(&mut self, bot: &BotRef, user_id: &str) -> Option<Row> {
        let entry = self.bots.get_mut(&key(bot))?;
        let row = entry.contacts.remove(user_id);
        if entry.contacts.is_empty() {
            self.bots.remove(&key(bot));
        }
        row
    }

    /// 所有 Bot 的所有行。
    pub fn all(&self) -> impl Iterator<Item = (&BotRef, &str, &Row)> {
        self.bots.values().flat_map(|rows| {
            rows.bot.iter().flat_map(move |bot| {
                rows.contacts
                    .iter()
                    .map(move |(user, row)| (bot, user.as_str(), row))
            })
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("读写联系人记录 {path} 失败：{source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("联系人记录 {path} 已损坏：{message}")]
    Corrupt { path: PathBuf, message: String },
}

pub fn read(path: &Path) -> Result<Stored, StoreError> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| StoreError::Corrupt {
            path: path.to_owned(),
            message: e.to_string(),
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Stored::default()),
        Err(source) => Err(StoreError::Io {
            path: path.to_owned(),
            source,
        }),
    }
}

/// 整份重写：先写临时文件再改名，断电不留半份记录（同 `core/peers.json`）。
pub fn write(path: &Path, stored: &Stored) -> Result<(), StoreError> {
    let io = |source| StoreError::Io {
        path: path.to_owned(),
        source,
    };
    let text = serde_json::to_vec_pretty(stored).map_err(|e| StoreError::Corrupt {
        path: path.to_owned(),
        message: e.to_string(),
    })?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(io)?;
    }
    let tmp = path.with_extension("json.tmp");
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp)
        .map_err(io)?;
    file.write_all(&text).map_err(io)?;
    file.sync_all().map_err(io)?;
    std::fs::rename(&tmp, path).map_err(io)
}
