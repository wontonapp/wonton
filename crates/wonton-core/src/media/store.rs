//! 媒体任务、播放会话、队列与配额账本的 SQLite 存储（ADR-0113 第 3 节）。
//!
//! 写和按编号读都带 `person`，按人隔离在这里做；只有 `tasks(None)`、`sessions(None)` 是列所有人的，
//! `task_any` 是服务层手里只有任务号时的入口。
//! 带枚举的列（种类、目标、状态、逐项状态、播放计划）按 JSON 文本存，其余拆列；上传的逐项进度是单独
//! 一张表，改偏移只写一行，不重写整个任务。
//!
//! 库文件是 `media.sqlite3`，WAL 与外键打开。连接只有一条，外面套 `Mutex` 串行化：写都是短事务，
//! 排队比连接池简单，也符合“核心保持薄”。

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use serde::de::DeserializeOwned;
use wonton_protocol::media::TaskId;
use wonton_protocol::media::view::{QuotaEntry, QuotaView};
use wonton_protocol::media::{PlaybackSessionId, Queue};

mod schema;
mod sessions;
mod tasks;

use schema::{QueueRow, classify, migrate, queue_row};
pub use sessions::StoredSession;

#[cfg(test)]
pub(crate) use schema::SCHEMA_V1;

const DB_FILE: &str = "media.sqlite3";

/// 每人向提供方上传的总量上限。ADR-0113 第 3 节没定值，先按 100 GiB 收；M16 接策略时用它兜底。
pub const DEFAULT_PROVIDER_UPLOAD_LIMIT: u64 = 100 * 1024 * 1024 * 1024;

/// 迁移前没有提供方的上传账行落在这个空串提供方上（`schema.rs` 的 `MIGRATE_V2`）；
/// 这个人的第一笔上传落到具体提供方时接手它（`record_usage`）。
const LEGACY_PROVIDER: &str = "";

#[derive(Debug, thiserror::Error)]
pub enum MediaStoreError {
    #[error("读写媒体库 {path} 失败：{source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("媒体库 {path} 不是可用的 SQLite 数据库：{source}")]
    Corrupt {
        path: PathBuf,
        #[source]
        source: rusqlite::Error,
    },
    #[error("媒体库 {path} 操作失败：{source}")]
    Database {
        path: PathBuf,
        #[source]
        source: rusqlite::Error,
    },
    #[error("媒体库 {path} 的行不是合法 JSON（{column}）：{source}")]
    Json {
        path: PathBuf,
        column: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("媒体库 {path} 里的编号 {raw} 不合法")]
    InvalidId { path: PathBuf, raw: String },
    #[error("媒体库 {path} 的写锁被上一次 panic 弄坏了")]
    Poisoned { path: PathBuf },
    #[error("媒体库 {path} 里没有任务 {task}")]
    TaskNotFound { path: PathBuf, task: TaskId },
    #[error("媒体库 {path} 里的任务 {task} 是下载，不能加条目")]
    TaskNotUpload { path: PathBuf, task: TaskId },
    #[error("媒体库 {path} 里没有这条播放会话 {session}")]
    SessionNotFound {
        path: PathBuf,
        session: PlaybackSessionId,
    },
    #[error("媒体库 {path} 里 {person} 已经有一条活动会话")]
    ActiveSessionExists { path: PathBuf, person: String },
    #[error("配额 {ledger} 超限：已用 {used}，上限 {limit}，这次要 {want}")]
    QuotaExceeded {
        ledger: &'static str,
        used: u64,
        limit: u64,
        want: u64,
    },
    #[error("配额用量溢出：已用 {used}，这次要 {want}")]
    QuotaOverflow { used: u64, want: u64 },
    #[error("离线的配额按设备记，要指定设备")]
    MissingDevice,
    #[error("上传的配额按提供方记，要指定提供方")]
    MissingProvider,
    #[error("媒体库 {path} 的版本 {found} 比核心支持的 {supported} 新")]
    TooNew {
        path: PathBuf,
        found: i64,
        supported: i64,
    },
}

/// 两本配额账：向提供方上传、设备上的离线文件。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ledger {
    ProviderUpload,
    DeviceOffline,
}

impl Ledger {
    fn as_str(self) -> &'static str {
        match self {
            Self::ProviderUpload => "provider_upload",
            Self::DeviceOffline => "device_offline",
        }
    }

    /// 一本账按什么分行：上传按提供方，离线按设备；另一维存空串。
    fn keys<'a>(
        self,
        provider: Option<&'a str>,
        device: Option<&'a str>,
    ) -> Result<(&'a str, &'a str), MediaStoreError> {
        match self {
            Self::ProviderUpload => Ok((provider.ok_or(MediaStoreError::MissingProvider)?, "")),
            Self::DeviceOffline => Ok(("", device.ok_or(MediaStoreError::MissingDevice)?)),
        }
    }

    fn default_limit(self) -> Option<u64> {
        match self {
            Self::ProviderUpload => Some(DEFAULT_PROVIDER_UPLOAD_LIMIT),
            Self::DeviceOffline => None,
        }
    }
}

#[derive(Debug)]
pub struct Store {
    path: PathBuf,
    conn: Mutex<Connection>,
}

impl Store {
    /// 打开（或新建）`dir` 下的媒体库；目录不存在时建出来。
    pub fn open(dir: impl AsRef<Path>) -> Result<Self, MediaStoreError> {
        let dir = dir.as_ref();
        let path = dir.join(DB_FILE);
        fs::create_dir_all(dir).map_err(|source| MediaStoreError::Io {
            path: dir.to_owned(),
            source,
        })?;
        let conn = Connection::open(&path).map_err(|source| MediaStoreError::Database {
            path: path.clone(),
            source,
        })?;
        // 换 journal_mode 要拿锁，第二个进程同时开库会撞 BUSY，超时先设。
        conn.busy_timeout(Duration::from_secs(5))
            .map_err(|source| MediaStoreError::Database {
                path: path.clone(),
                source,
            })?;
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(|source| classify(&path, source))?;
        conn.pragma_update(None, "foreign_keys", "ON")
            .map_err(|source| MediaStoreError::Database {
                path: path.clone(),
                source,
            })?;
        migrate(&conn, &path)?;
        Ok(Self {
            path,
            conn: Mutex::new(conn),
        })
    }

    /// 库文件路径。
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub(super) fn conn(&self) -> Result<MutexGuard<'_, Connection>, MediaStoreError> {
        self.conn.lock().map_err(|_| MediaStoreError::Poisoned {
            path: self.path.clone(),
        })
    }

    pub(super) fn db_error(&self, source: rusqlite::Error) -> MediaStoreError {
        MediaStoreError::Database {
            path: self.path.clone(),
            source,
        }
    }

    pub(super) fn json(
        &self,
        column: &'static str,
        value: &impl Serialize,
    ) -> Result<String, MediaStoreError> {
        serde_json::to_string(value).map_err(|source| MediaStoreError::Json {
            path: self.path.clone(),
            column,
            source,
        })
    }

    pub(super) fn read_json<T: DeserializeOwned>(
        &self,
        column: &'static str,
        text: &str,
    ) -> Result<T, MediaStoreError> {
        serde_json::from_str(text).map_err(|source| MediaStoreError::Json {
            path: self.path.clone(),
            column,
            source,
        })
    }

    pub(super) fn parse_id(&self, raw: &str) -> Result<TaskId, MediaStoreError> {
        TaskId::parse_str(raw).map_err(|_| MediaStoreError::InvalidId {
            path: self.path.clone(),
            raw: raw.to_owned(),
        })
    }

    pub fn queue(&self, person: &str) -> Result<Option<Queue>, MediaStoreError> {
        let conn = self.conn()?;
        let row: Option<QueueRow> = conn
            .query_row(
                "SELECT revision, entries, cursor, mode, sleep_at_ms FROM queue WHERE person = ?",
                params![person],
                queue_row,
            )
            .optional()
            .map_err(|source| self.db_error(source))?;
        row.map(|row| {
            Ok(Queue {
                revision: row.revision as u64,
                entries: self.read_json("entries", &row.entries)?,
                cursor: row.cursor.map(|value| value as usize),
                mode: self.read_json("mode", &row.mode)?,
                sleep_at_ms: row.sleep_at_ms.map(|value| value as u64),
            })
        })
        .transpose()
    }

    pub fn save_queue(&self, person: &str, queue: &Queue) -> Result<(), MediaStoreError> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO queue (person, revision, entries, cursor, mode, sleep_at_ms)
             VALUES (?, ?, ?, ?, ?, ?)
             ON CONFLICT(person) DO UPDATE SET
                 revision = excluded.revision, entries = excluded.entries, cursor = excluded.cursor,
                 mode = excluded.mode, sleep_at_ms = excluded.sleep_at_ms",
            params![
                person,
                queue.revision,
                self.json("entries", &queue.entries)?,
                queue.cursor.map(|value| value as i64),
                self.json("mode", &queue.mode)?,
                queue.sleep_at_ms,
            ],
        )
        .map_err(|source| self.db_error(source))?;
        Ok(())
    }

    /// 两本账的用量与上限，上传跨提供方、离线跨设备各自合计；没记过账的人是两项零，不建行。
    pub fn quota(&self, person: &str) -> Result<QuotaView, MediaStoreError> {
        let conn = self.conn()?;
        self.read_quota(&conn, person)
    }

    /// 记用量，超上限就整笔记不动。上传按提供方、离线按设备分开记；返回记完的合计。
    pub fn add_quota(
        &self,
        person: &str,
        ledger: Ledger,
        provider: Option<&str>,
        device: Option<&str>,
        bytes: u64,
    ) -> Result<QuotaView, MediaStoreError> {
        let (provider, device) = ledger.keys(provider, device)?;
        let mut conn = self.conn()?;
        let tx = conn.transaction().map_err(|source| self.db_error(source))?;
        let (used, limit) = self
            .ledger_usage(&tx, person, ledger, provider, device)?
            .unwrap_or((0, ledger.default_limit()));
        let want_total = used
            .checked_add(bytes)
            .ok_or(MediaStoreError::QuotaOverflow { used, want: bytes })?;
        if let Some(limit) = limit
            && want_total > limit
        {
            return Err(MediaStoreError::QuotaExceeded {
                ledger: ledger.as_str(),
                used,
                limit,
                want: bytes,
            });
        }
        self.record_usage(&tx, person, ledger, provider, device, want_total, limit)?;
        let view = self.read_quota(&tx, person)?;
        tx.commit().map_err(|source| self.db_error(source))?;
        Ok(view)
    }

    /// 释放用量，最多减到 0。返回减完的合计。这一格没记过账就不建行，视图的上限才不会多出默认值。
    pub fn take_quota(
        &self,
        person: &str,
        ledger: Ledger,
        provider: Option<&str>,
        device: Option<&str>,
        bytes: u64,
    ) -> Result<QuotaView, MediaStoreError> {
        let (provider, device) = ledger.keys(provider, device)?;
        let mut conn = self.conn()?;
        let tx = conn.transaction().map_err(|source| self.db_error(source))?;
        if let Some((used, limit)) = self.ledger_usage(&tx, person, ledger, provider, device)? {
            let remaining = used.saturating_sub(bytes);
            self.record_usage(&tx, person, ledger, provider, device, remaining, limit)?;
        }
        let view = self.read_quota(&tx, person)?;
        tx.commit().map_err(|source| self.db_error(source))?;
        Ok(view)
    }

    /// 写一格账。上传账先删掉迁移留下的空提供方旧账行：它的用量由这一格接手，
    /// 留着会被 `read_quota` 与这一格一起合计两遍。
    #[allow(clippy::too_many_arguments)] // 一格账就是这几个字段，再包结构体只是把参数搬个家。
    fn record_usage(
        &self,
        tx: &rusqlite::Transaction<'_>,
        person: &str,
        ledger: Ledger,
        provider: &str,
        device: &str,
        used: u64,
        limit: Option<u64>,
    ) -> Result<(), MediaStoreError> {
        if ledger == Ledger::ProviderUpload && provider != LEGACY_PROVIDER {
            tx.execute(
                "DELETE FROM quota WHERE person = ? AND ledger = ? AND provider = ? AND device = ?",
                params![person, ledger.as_str(), LEGACY_PROVIDER, device],
            )
            .map_err(|source| self.db_error(source))?;
        }
        tx.execute(
            "INSERT INTO quota (person, ledger, provider, device, used, cap)
             VALUES (?, ?, ?, ?, ?, ?)
             ON CONFLICT(person, ledger, provider, device) DO UPDATE SET used = excluded.used",
            params![
                person,
                ledger.as_str(),
                provider,
                device,
                used as i64,
                limit.map(|value| value as i64),
            ],
        )
        .map_err(|source| self.db_error(source))?;
        Ok(())
    }

    /// 这一格的用量与上限；没记过账返回 `None`。上传账没有这一格时回落到迁移留下的旧账行。
    fn ledger_usage(
        &self,
        conn: &Connection,
        person: &str,
        ledger: Ledger,
        provider: &str,
        device: &str,
    ) -> Result<Option<(u64, Option<u64>)>, MediaStoreError> {
        if let Some(row) = self.quota_row(conn, person, ledger, provider, device)? {
            return Ok(Some(row));
        }
        if ledger == Ledger::ProviderUpload && provider != LEGACY_PROVIDER {
            return self.quota_row(conn, person, ledger, LEGACY_PROVIDER, device);
        }
        Ok(None)
    }

    fn quota_row(
        &self,
        conn: &Connection,
        person: &str,
        ledger: Ledger,
        provider: &str,
        device: &str,
    ) -> Result<Option<(u64, Option<u64>)>, MediaStoreError> {
        let row: Option<(i64, Option<i64>)> = conn
            .query_row(
                "SELECT used, cap FROM quota
                 WHERE person = ? AND ledger = ? AND provider = ? AND device = ?",
                params![person, ledger.as_str(), provider, device],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|source| self.db_error(source))?;
        Ok(row.map(|(used, limit)| (used as u64, limit.map(|value| value as u64))))
    }

    fn read_quota(&self, conn: &Connection, person: &str) -> Result<QuotaView, MediaStoreError> {
        let mut stmt = conn
            .prepare("SELECT ledger, used, cap FROM quota WHERE person = ?")
            .map_err(|source| self.db_error(source))?;
        let rows = stmt
            .query_map(params![person], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)? as u64,
                    row.get::<_, Option<i64>>(2)?,
                ))
            })
            .map_err(|source| self.db_error(source))?;
        let mut upload = QuotaEntry {
            used: 0,
            limit: Ledger::ProviderUpload.default_limit(),
        };
        let mut offline = QuotaEntry {
            used: 0,
            limit: None,
        };
        let mut saw_upload = false;
        let mut saw_offline = false;
        for row in rows {
            let (ledger, used, limit) = row.map_err(|source| self.db_error(source))?;
            match ledger.as_str() {
                "provider_upload" => {
                    upload.used = upload.used.saturating_add(used);
                    upload.limit = add_cap(saw_upload, upload.limit, limit);
                    saw_upload = true;
                }
                "device_offline" => {
                    offline.used = offline.used.saturating_add(used);
                    offline.limit = add_cap(saw_offline, offline.limit, limit);
                    saw_offline = true;
                }
                _ => {}
            }
        }
        Ok(QuotaView {
            provider_upload: upload,
            device_offline: offline,
        })
    }
}

/// 合计上限：第一行取它的值，之后都有限才相加，出现不限就整体不限。
fn add_cap(seen: bool, sum: Option<u64>, cap: Option<i64>) -> Option<u64> {
    match (seen, sum, cap) {
        (false, _, cap) => cap.map(|value| value as u64),
        (true, Some(sum), Some(cap)) => Some(sum.saturating_add(cap as u64)),
        _ => None,
    }
}
