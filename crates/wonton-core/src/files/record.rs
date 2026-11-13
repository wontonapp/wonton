//! 编号的记录：指向哪里、谁能看、什么时候过期。
//!
//! 每条一个 JSON 文件（0600），改一条不用重写全部；分享过的记录不过期，会一直累积（ADR-0020 的“生命周期”）。

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt;
use std::fs;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::{Mutex, RwLock};

use serde::{Deserialize, Serialize};
use wonton_protocol::{FileId, FileSource};

use super::share_fs::{ShareRef, Snapshot};
use crate::plugin::PluginId;
use crate::plugin::store::{
    StoreError, create_private_dir, read_json, remove_file, write_json, write_json_synced,
};

/// Bot 登记、没分享的编号的有效期，再次拿到同一个引用时重新算。
pub const BOT_TTL_SECS: u64 = 24 * 3600;
/// 过期的记录再留这么久：Bot 这时用旧编号，得到的是 `file_expired` 而不是 `file_unknown`。
const EXPIRED_KEEP_SECS: u64 = 24 * 3600;

/// A person or Bot in a file's explicit access set; Owner is the actual server owner.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum Accessor {
    Owner,
    User(String),
    Bot(String),
}

impl fmt::Display for Accessor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Owner => f.write_str("owner"),
            Self::User(name) => write!(f, "user:{name}"),
            Self::Bot(name) => write!(f, "bot:{name}"),
        }
    }
}

impl FromStr for Accessor {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, String> {
        if let Some(username) = text.strip_prefix("user:") {
            crate::private_home::path(std::path::Path::new(""), username)?;
            return Ok(Self::User(username.to_owned()));
        }
        match (text, text.strip_prefix("bot:")) {
            ("owner", _) => Ok(Self::Owner),
            (_, Some(name)) if !name.is_empty() => Ok(Self::Bot(name.to_owned())),
            _ => Err(format!("不认识的访问者 {text}")),
        }
    }
}

impl TryFrom<String> for Accessor {
    type Error = String;

    fn try_from(text: String) -> Result<Self, String> {
        text.parse()
    }
}

impl From<Accessor> for String {
    fn from(accessor: Accessor) -> Self {
        accessor.to_string()
    }
}

/// 字节从哪读（ADR-0020 的“编号和来源”），都不复制。
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Origin {
    /// 共享文件夹里的原文件。
    Share { reference: ShareRef },
    /// 插件的资源。`internal_url` 没有时用 `resources/read`。
    Plugin {
        plugin: PluginId,
        uri: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        internal_url: Option<String>,
    },
    /// Device upload in core/home/<username>/transfers; placed is the deletion snapshot.
    Upload {
        reference: ShareRef,
        placed: Snapshot,
        username: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileRecord {
    pub id: FileId,
    pub name: String,
    /// 登记或上传时声明的类型，不可信：下载时的类型由核心决定（[`super::content::served_type`]）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    /// 插件声明的大小，或者设备上传时 tus 声明的长度（创建后不变）；文件现在的大小看当前的文件。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub declared_size: Option<u64>,
    pub source: FileSource,
    pub origin: Origin,
    pub access: BTreeSet<Accessor>,
    /// 主人亲手授权的 Bot（App 调 `grants`）。它们不再依赖来源插件的授权，要收回就单独收（ADR-0020 的“鉴权”）。
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub owner_granted: BTreeSet<String>,
    /// 登记时文件的样子：文件登记后改过，Bot 的工具返回 `file_changed`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registered: Option<Snapshot>,
    pub created_at: u64,
    /// 没分享的 Bot 编号的过期时间；分享过的和设备上传的没有。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<u64>,
}

impl FileRecord {
    fn expired(&self, now: u64) -> bool {
        self.expires_at.is_some_and(|at| at <= now)
    }
}

/// 按编号查到的结果。
#[derive(Debug, PartialEq, Eq)]
pub enum Lookup {
    Found(FileRecord),
    /// 编号还记得，但没分享就过期了。设备看到的和没有这个编号一样，只有 Bot 的工具才说 `file_expired`。
    Expired(FileRecord),
    Unknown,
}

#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("读取编号记录目录 {path} 失败：{source}")]
    List {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// 编号记录：内存里一份，每次修改先落盘。
///
/// `writes` 把修改排成队，查找和修改算一步（[`Self::change`]）。写盘和 `fsync` 只持有它，不持有 `records`：写盘时读不用等。
/// 同步的文件操作，调用方放在阻塞线程上。
pub struct Registry {
    dir: PathBuf,
    writes: Mutex<()>,
    records: RwLock<Records>,
}

/// 全部记录，加一个“这个 Bot 对这个来源有哪些编号”的索引：Bot 每次拿到文件引用都要查，记录可以很多。
#[derive(Default)]
struct Records {
    by_id: HashMap<FileId, FileRecord>,
    by_bot: HashMap<(String, Origin), HashSet<FileId>>,
}

fn bot_key(record: &FileRecord) -> Option<(String, Origin)> {
    match &record.source {
        FileSource::Bot { name, .. } => Some((name.clone(), record.origin.clone())),
        _ => None,
    }
}

impl Records {
    fn insert(&mut self, record: FileRecord) {
        self.remove(&record.id);
        if let Some(key) = bot_key(&record) {
            self.by_bot
                .entry(key)
                .or_default()
                .insert(record.id.clone());
        }
        self.by_id.insert(record.id.clone(), record);
    }

    fn remove(&mut self, id: &FileId) {
        let Some(old) = self.by_id.remove(id) else {
            return;
        };
        if let Some(key) = bot_key(&old)
            && let Some(ids) = self.by_bot.get_mut(&key)
        {
            ids.remove(id);
            if ids.is_empty() {
                self.by_bot.remove(&key);
            }
        }
    }

    /// 这个 Bot 从 `plugin` 拿到这个来源时接着用的记录：同一个插件给过的优先，其次是没分享的。
    fn find_bot_record(
        &self,
        bot: &str,
        origin: &Origin,
        plugin: &str,
        now: u64,
    ) -> Option<&FileRecord> {
        let ids = self.by_bot.get(&(bot.to_owned(), origin.clone()))?;
        let live = || {
            ids.iter()
                .filter_map(|id| self.by_id.get(id))
                .filter(|record| !record.expired(now))
        };
        live()
            .find(|record| matches!(&record.source, FileSource::Bot { plugin: from, .. } if from == plugin))
            .or_else(|| live().find(|record| record.expires_at.is_some()))
    }
}

impl Registry {
    /// 读出目录里的全部记录。写入是原子的（先写临时文件再改名），读不懂的记录说明出了别的问题：启动失败，写明是哪个文件。
    pub fn open(dir: PathBuf) -> Result<Self, RegistryError> {
        create_private_dir(&dir)?;
        let list_error = |source| RegistryError::List {
            path: dir.clone(),
            source,
        };
        let mut records = Records::default();
        for entry in fs::read_dir(&dir).map_err(list_error)? {
            let path = entry.map_err(list_error)?.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            if let Some(record) = read_json::<FileRecord>(&path)? {
                records.insert(record);
            }
        }
        Ok(Self {
            dir,
            writes: Mutex::new(()),
            records: RwLock::new(records),
        })
    }

    fn path(&self, id: &FileId) -> PathBuf {
        self.dir.join(format!("{id}.json"))
    }

    pub fn get(&self, id: &FileId, now: u64) -> Lookup {
        let records = self.records.read().expect("编号记录锁中毒");
        match records.by_id.get(id) {
            Some(record) if record.expired(now) => Lookup::Expired(record.clone()),
            Some(record) => Lookup::Found(record.clone()),
            None => Lookup::Unknown,
        }
    }

    /// 看着当前的记录决定要不要写一条：`decide` 返回要写的记录和给调用方的结果。
    fn change<T>(
        &self,
        decide: impl FnOnce(&Records) -> (Option<FileRecord>, T),
    ) -> Result<T, RegistryError> {
        let _queue = self.writes.lock().expect("编号记录写队列中毒");
        let (record, result) = decide(&self.records.read().expect("编号记录锁中毒"));
        if let Some(record) = record {
            write_json(&self.path(&record.id), &record)?;
            self.records.write().expect("编号记录锁中毒").insert(record);
        }
        Ok(result)
    }

    /// 写入一条新记录或改过的记录。
    pub fn save(&self, record: FileRecord) -> Result<(), RegistryError> {
        self.change(|_| (Some(record), ()))
    }

    /// Bot 拿到一个引用：同一个 Bot 上次的记录还在就用 `renew` 改它，否则写 `fresh` 给的新记录。过期了的不算：Bot 再找一次就有
    /// 新编号。分享过的记录只有同一个插件再给才算：卡片上的来源是主人判断内容可不可信的依据，不能事后换，新内容也不能挂着旧来源再分享；
    /// 授权也按来源核对，留着旧编号的话，原来的插件没了授权，Bot 就永远用不了这个文件。
    pub fn register_bot(
        &self,
        bot: &str,
        origin: &Origin,
        plugin: &str,
        now: u64,
        renew: impl FnOnce(&mut FileRecord),
        fresh: impl FnOnce() -> FileRecord,
    ) -> Result<FileRecord, RegistryError> {
        self.change(|records| {
            let record = match records.find_bot_record(bot, origin, plugin, now) {
                Some(old) => {
                    let mut old = old.clone();
                    renew(&mut old);
                    old
                }
                None => fresh(),
            };
            (Some(record.clone()), record)
        })
    }

    /// 把访问者加进范围，返回改过的记录。加了主人就是分享过了，不再过期。
    pub fn grant(
        &self,
        id: &FileId,
        accessor: Accessor,
        now: u64,
    ) -> Result<Option<FileRecord>, RegistryError> {
        self.change(|records| {
            let Some(record) = records.by_id.get(id).filter(|r| !r.expired(now)) else {
                return (None, None);
            };
            let mut record = record.clone();
            if !matches!(accessor, Accessor::Bot(_)) {
                record.expires_at = None;
            }
            record.access.insert(accessor);
            (Some(record.clone()), Some(record))
        })
    }

    /// 主人亲手把这个 Bot 加进访问范围。
    pub fn grant_by_owner(
        &self,
        id: &FileId,
        bot: &str,
        now: u64,
    ) -> Result<Option<FileRecord>, RegistryError> {
        self.change(|records| {
            let Some(record) = records.by_id.get(id).filter(|r| !r.expired(now)) else {
                return (None, None);
            };
            let mut record = record.clone();
            record.access.insert(Accessor::Bot(bot.to_owned()));
            record.owner_granted.insert(bot.to_owned());
            (Some(record.clone()), Some(record))
        })
    }

    /// 把这个 Bot 移出访问范围。不在范围里时什么都不做。
    pub fn revoke_bot(&self, id: &FileId, bot: &str) -> Result<Option<FileRecord>, RegistryError> {
        self.change(|records| {
            let Some(record) = records.by_id.get(id) else {
                return (None, None);
            };
            let mut record = record.clone();
            record.access.remove(&Accessor::Bot(bot.to_owned()));
            record.owner_granted.remove(bot);
            (Some(record.clone()), Some(record))
        })
    }

    pub fn remove(&self, id: &FileId) -> Result<(), RegistryError> {
        let _queue = self.writes.lock().expect("编号记录写队列中毒");
        remove_file(&self.path(id))?;
        self.records.write().expect("编号记录锁中毒").remove(id);
        Ok(())
    }

    pub(super) fn remove_home_uploads(&self, username: &str) -> Result<(), RegistryError> {
        let _queue = self.writes.lock().expect("编号记录写队列中毒");
        let ids: Vec<FileId> = self
            .records
            .read()
            .expect("编号记录锁中毒")
            .by_id
            .values()
            .filter(|record| matches!(&record.origin, Origin::Upload { username: owner, .. } if owner == username))
            .map(|record| record.id.clone())
            .collect();
        for id in ids {
            remove_file(&self.path(&id))?;
            self.records.write().expect("编号记录锁中毒").remove(&id);
        }
        Ok(())
    }

    /// 写一条记录，返回目录项落盘了没有。写已经生效了，读得到；后面的步骤要删掉记录以外的东西时，要看这个结果。
    pub fn save_synced(&self, record: FileRecord) -> Result<bool, RegistryError> {
        let _queue = self.writes.lock().expect("编号记录写队列中毒");
        let synced = write_json_synced(&self.path(&record.id), &record)?;
        self.records.write().expect("编号记录锁中毒").insert(record);
        Ok(synced)
    }

    /// 上次没落盘的记录重新写一次，返回这次落盘了没有。记录已经没了就没什么可丢的。
    pub fn resave(&self, id: &FileId) -> Result<bool, RegistryError> {
        let _queue = self.writes.lock().expect("编号记录写队列中毒");
        let records = self.records.read().expect("编号记录锁中毒");
        let Some(record) = records.by_id.get(id) else {
            return Ok(true);
        };
        Ok(write_json_synced(&self.path(id), record)?)
    }

    /// 到期了的设备上传编号：链接一直没发出去，文件和记录都要删。
    pub fn expired_uploads(&self, now: u64) -> Vec<FileId> {
        let records = self.records.read().expect("编号记录锁中毒");
        let mut ids: Vec<FileId> = records
            .by_id
            .values()
            .filter(|r| matches!(r.origin, Origin::Upload { .. }) && r.expired(now))
            .map(|r| r.id.clone())
            .collect();
        ids.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        ids
    }

    /// 清掉过期时间，已经过期了也一样：到期那天名字上已经是主人的文件，就不再自动删。
    pub fn keep(&self, id: &FileId) -> Result<(), RegistryError> {
        self.change(|records| {
            let Some(record) = records.by_id.get(id).cloned() else {
                return (None, ());
            };
            if record.expires_at.is_none() {
                return (None, ());
            }
            let record = FileRecord {
                expires_at: None,
                ..record
            };
            (Some(record), ())
        })
    }

    /// 删掉过期够久的记录，返回删了几条。
    pub fn purge_expired(&self, now: u64) -> Result<usize, RegistryError> {
        let stale: Vec<FileId> = {
            let records = self.records.read().expect("编号记录锁中毒");
            records
                .by_id
                .values()
                .filter(|r| {
                    // 上传删不掉时记录要留着，否则文件成了没人管的孤儿。
                    !matches!(r.origin, Origin::Upload { .. })
                        && r.expires_at
                            .is_some_and(|at| at.saturating_add(EXPIRED_KEEP_SECS) <= now)
                })
                .map(|r| r.id.clone())
                .collect()
        };
        for id in &stale {
            self.remove(id)?;
        }
        Ok(stale.len())
    }
}

#[cfg(test)]
mod tests;
