//! 媒体任务、播放会话、队列与配额账本的 SQLite 存储（ADR-0113 第 3 节）。
//!
//! 写和按编号读都带 `person`，按人隔离在这里做；只有 `tasks(None)`、`sessions(None)` 是列所有人的。
//! 带枚举的列（种类、目标、状态、逐项状态、播放计划）按 JSON 文本存，其余拆列；上传的逐项进度是单独
//! 一张表，改偏移只写一行，不重写整个任务。
//!
//! 库文件是 `media.sqlite3`，WAL 与外键打开。连接只有一条，外面套 `Mutex` 串行化：写都是短事务，
//! 排队比连接池简单，也符合“核心保持薄”。

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use rusqlite::types::Value;
use rusqlite::{Connection, OptionalExtension, params, params_from_iter};
use serde::Serialize;
use serde::de::DeserializeOwned;
use wonton_protocol::media::TaskId;
use wonton_protocol::media::view::{QuotaEntry, QuotaView};
use wonton_protocol::media::{
    MediaItemRef, MediaTask, PlaybackPlan, PlaybackSession, PlaybackSessionId, Queue, TaskKind,
    TaskState, TaskTarget, UploadItem,
};

mod schema;

use schema::{
    QueueRow, SessionRow, TaskRow, classify, migrate, queue_row, row_to_task, session_row,
};

#[cfg(test)]
pub(crate) use schema::SCHEMA_V1;

const DB_FILE: &str = "media.sqlite3";

/// 每人向提供方上传的总量上限。ADR-0113 第 3 节没定值，先按 100 GiB 收；M16 接策略时用它兜底。
pub const DEFAULT_PROVIDER_UPLOAD_LIMIT: u64 = 100 * 1024 * 1024 * 1024;

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
    #[error("媒体库 {path} 里没有这个人的任务 {task}")]
    TaskNotFound { path: PathBuf, task: TaskId },
    #[error("媒体库 {path} 里的任务 {task} 是下载，不能加条目")]
    TaskNotUpload { path: PathBuf, task: TaskId },
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

    fn conn(&self) -> Result<MutexGuard<'_, Connection>, MediaStoreError> {
        self.conn.lock().map_err(|_| MediaStoreError::Poisoned {
            path: self.path.clone(),
        })
    }

    fn db_error(&self, source: rusqlite::Error) -> MediaStoreError {
        MediaStoreError::Database {
            path: self.path.clone(),
            source,
        }
    }

    fn json(
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

    fn read_json<T: DeserializeOwned>(
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

    fn parse_id(&self, raw: &str) -> Result<TaskId, MediaStoreError> {
        TaskId::parse_str(raw).map_err(|_| MediaStoreError::InvalidId {
            path: self.path.clone(),
            raw: raw.to_owned(),
        })
    }

    /// 存一条任务；上传的逐项条目和目标一起落库，目标列里只留空的条目表。
    pub fn create_task(&self, task: &MediaTask) -> Result<(), MediaStoreError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction().map_err(|source| self.db_error(source))?;
        let target = match &task.target {
            TaskTarget::Upload(_) => TaskTarget::Upload(Vec::new()),
            other => other.clone(),
        };
        tx.execute(
            "INSERT INTO tasks (id, person, device, kind, target, state, state_tag, progress,
                                retry_count, retry_of, created_at, partial_failures)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                task.id.to_string(),
                task.person,
                task.device,
                kind_raw(&task.kind),
                self.json("target", &target)?,
                self.json("state", &task.state)?,
                task.state.as_str(),
                self.json("progress", &task.progress)?,
                task.retry_count,
                task.retry_of.map(|id| id.to_string()),
                task.created_at,
                self.json("partial_failures", &task.partial_failures)?,
            ],
        )
        .map_err(|source| self.db_error(source))?;
        if let TaskTarget::Upload(items) = &task.target {
            self.insert_items(&tx, &task.person, task.id, items, 0)?;
        }
        tx.commit().map_err(|source| self.db_error(source))?;
        Ok(())
    }

    /// 按人读一条任务；不是这个人的行当作没有。
    pub fn task(&self, person: &str, id: TaskId) -> Result<Option<MediaTask>, MediaStoreError> {
        let conn = self.conn()?;
        match self.task_row(&conn, person, id)? {
            Some(row) => Ok(Some(self.assemble_task(&conn, row)?)),
            None => Ok(None),
        }
    }

    /// 列任务；`person` 为空列所有人的，`kind`/`state` 为空不筛。
    pub fn tasks(
        &self,
        person: Option<&str>,
        kind: Option<&TaskKind>,
        state: Option<&TaskState>,
    ) -> Result<Vec<MediaTask>, MediaStoreError> {
        let conn = self.conn()?;
        let mut sql = String::from(
            "SELECT id, person, device, kind, target, state, progress, retry_count, retry_of,
                    created_at, partial_failures FROM tasks",
        );
        let mut filters: Vec<&str> = Vec::new();
        let mut values: Vec<Value> = Vec::new();
        if let Some(person) = person {
            filters.push("person = ?");
            values.push(Value::Text(person.to_owned()));
        }
        if let Some(kind) = kind {
            filters.push("kind = ?");
            values.push(Value::Text(kind_raw(kind).to_owned()));
        }
        if let Some(state) = state {
            filters.push("state_tag = ?");
            values.push(Value::Text(state.as_str().to_owned()));
        }
        if !filters.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&filters.join(" AND "));
        }
        sql.push_str(" ORDER BY created_at, id");
        let mut stmt = conn.prepare(&sql).map_err(|source| self.db_error(source))?;
        let rows: Vec<TaskRow> = stmt
            .query_map(params_from_iter(values), row_to_task)
            .map_err(|source| self.db_error(source))?
            .collect::<Result<_, _>>()
            .map_err(|source| self.db_error(source))?;
        rows.into_iter()
            .map(|row| self.assemble_task(&conn, row))
            .collect()
    }

    /// 回写任务的状态、进度等普通字段；返回是否落到了这个人的行上。
    ///
    /// **不写逐项条目**：`target` 里带的 `UploadItem` 会被忽略，条目只认 `create_task`、`enqueue_items`、
    /// `save_item` 写的行。改了某一项要另外调 [`Self::save_item`]。
    pub fn save_task(&self, task: &MediaTask) -> Result<bool, MediaStoreError> {
        let conn = self.conn()?;
        let target = match &task.target {
            TaskTarget::Upload(_) => TaskTarget::Upload(Vec::new()),
            other => other.clone(),
        };
        let changed = conn
            .execute(
                "UPDATE tasks SET device = ?, kind = ?, target = ?, state = ?, state_tag = ?,
                                  progress = ?, retry_count = ?, retry_of = ?, created_at = ?,
                                  partial_failures = ?
                 WHERE id = ? AND person = ?",
                params![
                    task.device,
                    kind_raw(&task.kind),
                    self.json("target", &target)?,
                    self.json("state", &task.state)?,
                    task.state.as_str(),
                    self.json("progress", &task.progress)?,
                    task.retry_count,
                    task.retry_of.map(|id| id.to_string()),
                    task.created_at,
                    self.json("partial_failures", &task.partial_failures)?,
                    task.id.to_string(),
                    task.person,
                ],
            )
            .map_err(|source| self.db_error(source))?;
        Ok(changed > 0)
    }

    /// 往上传任务末尾加条目。
    pub fn enqueue_items(
        &self,
        person: &str,
        task: TaskId,
        items: &[UploadItem],
    ) -> Result<(), MediaStoreError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction().map_err(|source| self.db_error(source))?;
        let kind: Option<String> = tx
            .query_row(
                "SELECT kind FROM tasks WHERE id = ? AND person = ?",
                params![task.to_string(), person],
                |row| row.get(0),
            )
            .optional()
            .map_err(|source| self.db_error(source))?;
        match kind.as_deref() {
            Some("upload") => {}
            Some(_) => {
                return Err(MediaStoreError::TaskNotUpload {
                    path: self.path.clone(),
                    task,
                });
            }
            None => {
                return Err(MediaStoreError::TaskNotFound {
                    path: self.path.clone(),
                    task,
                });
            }
        }
        let start: i64 = tx
            .query_row(
                "SELECT COALESCE(MAX(position) + 1, 0) FROM task_items
                 WHERE person = ? AND task_id = ?",
                params![person, task.to_string()],
                |row| row.get(0),
            )
            .map_err(|source| self.db_error(source))?;
        self.insert_items(&tx, person, task, items, start)?;
        tx.commit().map_err(|source| self.db_error(source))?;
        Ok(())
    }

    /// 任务里每一项的已收字节，按加入顺序。
    pub fn item_offsets(
        &self,
        person: &str,
        task: TaskId,
    ) -> Result<Vec<(String, u64)>, MediaStoreError> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare(
                "SELECT item_id, upload_offset FROM task_items
                 WHERE person = ? AND task_id = ? ORDER BY position, item_id",
            )
            .map_err(|source| self.db_error(source))?;
        let rows = stmt
            .query_map(params![person, task.to_string()], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as u64))
            })
            .map_err(|source| self.db_error(source))?;
        rows.collect::<Result<_, _>>()
            .map_err(|source| self.db_error(source))
    }

    /// 回写一项的偏移与状态；返回是否落到了这个人的行上。
    pub fn save_item(
        &self,
        person: &str,
        task: TaskId,
        item: &UploadItem,
    ) -> Result<bool, MediaStoreError> {
        let conn = self.conn()?;
        let changed = conn
            .execute(
                "UPDATE task_items SET path = ?, size = ?, checksum = ?, taken_at = ?, state = ?,
                                       upload_offset = ?
                 WHERE person = ? AND task_id = ? AND item_id = ?",
                params![
                    item.path,
                    item.size,
                    item.checksum,
                    item.taken_at,
                    self.json("state", &item.state)?,
                    item.upload_offset,
                    person,
                    task.to_string(),
                    item.item_id,
                ],
            )
            .map_err(|source| self.db_error(source))?;
        Ok(changed > 0)
    }

    pub fn create_session(&self, session: &PlaybackSession) -> Result<(), MediaStoreError> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO sessions (id, person, device, item, state, state_tag, position_ms, plan)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                session.id.to_string(),
                session.person,
                session.device,
                self.json("item", &session.item)?,
                self.json("state", &session.state)?,
                session.state.as_str(),
                session.position_ms,
                session
                    .plan
                    .as_ref()
                    .map(|plan| self.json("plan", plan))
                    .transpose()?,
            ],
        )
        .map_err(|source| self.db_error(source))?;
        Ok(())
    }

    pub fn session(
        &self,
        person: &str,
        id: PlaybackSessionId,
    ) -> Result<Option<PlaybackSession>, MediaStoreError> {
        let conn = self.conn()?;
        let row: Option<SessionRow> = conn
            .query_row(
                "SELECT id, person, device, item, state, position_ms, plan FROM sessions
                 WHERE id = ? AND person = ?",
                params![id.to_string(), person],
                session_row,
            )
            .optional()
            .map_err(|source| self.db_error(source))?;
        row.map(|row| self.assemble_session(row)).transpose()
    }

    /// 列会话，按建立的先后；`person` 为空列所有人的。表里另存了状态标签，M9 用它找活动会话。
    pub fn sessions(&self, person: Option<&str>) -> Result<Vec<PlaybackSession>, MediaStoreError> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, person, device, item, state, position_ms, plan FROM sessions
                 WHERE ? IS NULL OR person = ? ORDER BY rowid",
            )
            .map_err(|source| self.db_error(source))?;
        let rows: Vec<SessionRow> = stmt
            .query_map(params![person, person], session_row)
            .map_err(|source| self.db_error(source))?
            .collect::<Result<_, _>>()
            .map_err(|source| self.db_error(source))?;
        rows.into_iter()
            .map(|row| self.assemble_session(row))
            .collect()
    }

    /// 回写会话的位置与计划；返回是否落到了这个人的行上。
    pub fn save_session(&self, session: &PlaybackSession) -> Result<bool, MediaStoreError> {
        let conn = self.conn()?;
        let changed = conn
            .execute(
                "UPDATE sessions SET device = ?, item = ?, state = ?, state_tag = ?, position_ms = ?,
                                     plan = ?
                 WHERE id = ? AND person = ?",
                params![
                    session.device,
                    self.json("item", &session.item)?,
                    self.json("state", &session.state)?,
                    session.state.as_str(),
                    session.position_ms,
                    session
                        .plan
                        .as_ref()
                        .map(|plan| self.json("plan", plan))
                        .transpose()?,
                    session.id.to_string(),
                    session.person,
                ],
            )
            .map_err(|source| self.db_error(source))?;
        Ok(changed > 0)
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
        let (used, limit) = self.ledger_usage(&tx, person, ledger, provider, device)?;
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
        tx.execute(
            "INSERT INTO quota (person, ledger, provider, device, used, cap)
             VALUES (?, ?, ?, ?, ?, ?)
             ON CONFLICT(person, ledger, provider, device) DO UPDATE SET used = excluded.used",
            params![
                person,
                ledger.as_str(),
                provider,
                device,
                want_total as i64,
                limit.map(|value| value as i64),
            ],
        )
        .map_err(|source| self.db_error(source))?;
        let view = self.read_quota(&tx, person)?;
        tx.commit().map_err(|source| self.db_error(source))?;
        Ok(view)
    }

    /// 释放用量，最多减到 0。返回减完的合计。
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
        let (used, limit) = self.ledger_usage(&tx, person, ledger, provider, device)?;
        let remaining = used.saturating_sub(bytes);
        tx.execute(
            "INSERT INTO quota (person, ledger, provider, device, used, cap)
             VALUES (?, ?, ?, ?, ?, ?)
             ON CONFLICT(person, ledger, provider, device) DO UPDATE SET used = excluded.used",
            params![
                person,
                ledger.as_str(),
                provider,
                device,
                remaining as i64,
                limit.map(|value| value as i64),
            ],
        )
        .map_err(|source| self.db_error(source))?;
        let view = self.read_quota(&tx, person)?;
        tx.commit().map_err(|source| self.db_error(source))?;
        Ok(view)
    }

    fn ledger_usage(
        &self,
        conn: &Connection,
        person: &str,
        ledger: Ledger,
        provider: &str,
        device: &str,
    ) -> Result<(u64, Option<u64>), MediaStoreError> {
        let row: Option<(i64, Option<i64>)> = conn
            .query_row(
                "SELECT used, cap FROM quota
                 WHERE person = ? AND ledger = ? AND provider = ? AND device = ?",
                params![person, ledger.as_str(), provider, device],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|source| self.db_error(source))?;
        Ok(match row {
            Some((used, limit)) => (used as u64, limit.map(|value| value as u64)),
            None => (0, ledger.default_limit()),
        })
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

    fn insert_items(
        &self,
        conn: &Connection,
        person: &str,
        task: TaskId,
        items: &[UploadItem],
        start: i64,
    ) -> Result<(), MediaStoreError> {
        let mut stmt = conn
            .prepare(
                "INSERT INTO task_items (person, task_id, item_id, position, path, size, checksum,
                                         taken_at, state, upload_offset)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .map_err(|source| self.db_error(source))?;
        for (offset, item) in items.iter().enumerate() {
            stmt.execute(params![
                person,
                task.to_string(),
                item.item_id,
                start + offset as i64,
                item.path,
                item.size,
                item.checksum,
                item.taken_at,
                self.json("state", &item.state)?,
                item.upload_offset,
            ])
            .map_err(|source| self.db_error(source))?;
        }
        Ok(())
    }

    fn task_row(
        &self,
        conn: &Connection,
        person: &str,
        id: TaskId,
    ) -> Result<Option<TaskRow>, MediaStoreError> {
        conn.query_row(
            "SELECT id, person, device, kind, target, state, progress, retry_count, retry_of,
                    created_at, partial_failures FROM tasks WHERE id = ? AND person = ?",
            params![id.to_string(), person],
            row_to_task,
        )
        .optional()
        .map_err(|source| self.db_error(source))
    }

    fn assemble_task(&self, conn: &Connection, row: TaskRow) -> Result<MediaTask, MediaStoreError> {
        let mut target: TaskTarget = self.read_json("target", &row.target)?;
        if let TaskTarget::Upload(items) = &mut target {
            *items = self.items(conn, &row.person, &row.id)?;
        }
        Ok(MediaTask {
            id: self.parse_id(&row.id)?,
            kind: kind_from_raw(&row.kind),
            person: row.person,
            device: row.device,
            target,
            state: self.read_json("state", &row.state)?,
            progress: self.read_json("progress", &row.progress)?,
            retry_count: row.retry_count as u8,
            retry_of: row
                .retry_of
                .as_deref()
                .map(|raw| self.parse_id(raw))
                .transpose()?,
            created_at: row.created_at as u64,
            partial_failures: self.read_json("partial_failures", &row.partial_failures)?,
        })
    }

    fn items(
        &self,
        conn: &Connection,
        person: &str,
        task: &str,
    ) -> Result<Vec<UploadItem>, MediaStoreError> {
        let mut stmt = conn
            .prepare(
                "SELECT item_id, path, size, checksum, taken_at, state, upload_offset
                 FROM task_items WHERE person = ? AND task_id = ? ORDER BY position, item_id",
            )
            .map_err(|source| self.db_error(source))?;
        let rows = stmt
            .query_map(params![person, task], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)? as u64,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<i64>>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, i64>(6)? as u64,
                ))
            })
            .map_err(|source| self.db_error(source))?;
        rows.map(|row| {
            let (item_id, path, size, checksum, taken_at, state, upload_offset) =
                row.map_err(|source| self.db_error(source))?;
            Ok(UploadItem {
                item_id,
                path,
                size,
                checksum,
                taken_at: taken_at.map(|value| value as u64),
                state: self.read_json("state", &state)?,
                upload_offset,
            })
        })
        .collect()
    }

    fn assemble_session(&self, row: SessionRow) -> Result<PlaybackSession, MediaStoreError> {
        Ok(PlaybackSession {
            id: self.parse_id(&row.id)?,
            person: row.person,
            device: row.device,
            item: self.read_json::<MediaItemRef>("item", &row.item)?,
            state: self.read_json("state", &row.state)?,
            position_ms: row.position_ms as u64,
            plan: row
                .plan
                .as_deref()
                .map(|text| self.read_json::<PlaybackPlan>("plan", text))
                .transpose()?,
        })
    }
}

/// 种类按原值存（没有载荷），筛选直接比字符串；带载荷的状态才走 JSON。
fn kind_raw(kind: &TaskKind) -> &str {
    match kind {
        TaskKind::Upload => "upload",
        TaskKind::Download => "download",
        TaskKind::Other(raw) => raw,
    }
}

fn kind_from_raw(raw: &str) -> TaskKind {
    match raw {
        "upload" => TaskKind::Upload,
        "download" => TaskKind::Download,
        other => TaskKind::Other(other.to_owned()),
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
