//! 媒体库的建表与升级语句、行到结构体的搬运。
//!
//! 结构版本记在 `PRAGMA user_version`。改结构时改 [`SCHEMA`] 并把 `CURRENT_VERSION` 加一，不写迁移。

use std::path::Path;

use rusqlite::Row;

use super::MediaStoreError;

pub(crate) const CURRENT_VERSION: i64 = 2;

/// 建表。有外部用户之前结构直接改、不写迁移（AGENTS.md“兼容”）：改了结构的库按版本号拒开，删掉重建。
pub(super) const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS tasks (
    id TEXT PRIMARY KEY,
    person TEXT NOT NULL,
    device TEXT NOT NULL,
    kind TEXT NOT NULL,
    target TEXT NOT NULL,
    state TEXT NOT NULL,
    state_tag TEXT NOT NULL,
    progress TEXT NOT NULL,
    retry_count INTEGER NOT NULL,
    retry_of TEXT,
    created_at INTEGER NOT NULL,
    partial_failures TEXT NOT NULL,
    -- 下一次自动重试的时间；等网络的任务按它找
    next_attempt_at INTEGER,
    provider TEXT
);
CREATE INDEX IF NOT EXISTS tasks_by_person ON tasks(person, created_at);
CREATE INDEX IF NOT EXISTS tasks_due ON tasks(state_tag, next_attempt_at);
-- 逐项条目单独成表，偏移一次只改一行。
CREATE TABLE IF NOT EXISTS task_items (
    person TEXT NOT NULL,
    task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    item_id TEXT NOT NULL,
    position INTEGER NOT NULL,
    path TEXT NOT NULL,
    size INTEGER NOT NULL,
    checksum TEXT,
    taken_at INTEGER,
    state TEXT NOT NULL,
    upload_offset INTEGER NOT NULL,
    PRIMARY KEY (task_id, item_id)
);
CREATE INDEX IF NOT EXISTS task_items_by_person ON task_items(person, task_id);
CREATE TABLE IF NOT EXISTS sessions (
    id TEXT PRIMARY KEY,
    person TEXT NOT NULL,
    device TEXT NOT NULL,
    item TEXT NOT NULL,
    state TEXT NOT NULL,
    -- 和 state 一起写；找这个人的活动会话用
    state_tag TEXT NOT NULL,
    position_ms INTEGER NOT NULL,
    plan TEXT,
    stop_reason TEXT,
    -- 每次写加一，按读到的版本比较再写
    version INTEGER NOT NULL DEFAULT 0,
    provider TEXT,
    -- 起播时的设备档
    profile TEXT
);
CREATE INDEX IF NOT EXISTS sessions_by_person ON sessions(person);
-- 一人同时只能有一条活动会话（停因为空的）。
CREATE UNIQUE INDEX IF NOT EXISTS sessions_one_active ON sessions(person) WHERE stop_reason IS NULL;
CREATE TABLE IF NOT EXISTS queue (
    person TEXT PRIMARY KEY,
    revision INTEGER NOT NULL,
    entries TEXT NOT NULL,
    cursor INTEGER,
    mode TEXT NOT NULL,
    sleep_at_ms INTEGER
);
-- 上传按提供方记（device 存空串），离线按设备记（provider 存空串）。
CREATE TABLE IF NOT EXISTS quota (
    person TEXT NOT NULL,
    ledger TEXT NOT NULL,
    provider TEXT NOT NULL,
    device TEXT NOT NULL,
    used INTEGER NOT NULL,
    cap INTEGER,
    PRIMARY KEY (person, ledger, provider, device)
);
-- 上传项在提供方那边的号（ADR-0113 第 2 节“字节路径”），只在核心。
CREATE TABLE IF NOT EXISTS uploads (
    person TEXT NOT NULL,
    task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    item_id TEXT NOT NULL,
    provider TEXT NOT NULL,
    upload_id TEXT NOT NULL,
    PRIMARY KEY (task_id, item_id)
);
-- 发给设备的命令（ADR-0113 第 5 节）；回执了的等设备下次拉时清。
CREATE TABLE IF NOT EXISTS commands (
    command_id TEXT PRIMARY KEY,
    device TEXT NOT NULL,
    seq INTEGER NOT NULL,
    body TEXT NOT NULL,
    acked INTEGER NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS commands_by_device ON commands(device, seq);
-- 每台设备发到第几条；命令清掉了也不回退。
CREATE TABLE IF NOT EXISTS command_seq (
    device TEXT PRIMARY KEY,
    last INTEGER NOT NULL
);
";

/// 新库建表；版本不是 [`CURRENT_VERSION`] 的库拒绝打开，不替它补。
pub(super) fn migrate(conn: &rusqlite::Connection, path: &Path) -> Result<(), MediaStoreError> {
    let found: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|source| classify(path, source))?;
    if found != 0 && found != CURRENT_VERSION {
        return Err(MediaStoreError::UnsupportedVersion {
            path: path.to_owned(),
            found,
            supported: CURRENT_VERSION,
        });
    }
    if found == CURRENT_VERSION {
        return Ok(());
    }
    let tx = conn
        .unchecked_transaction()
        .map_err(|source| classify(path, source))?;
    tx.execute_batch(SCHEMA)
        .map_err(|source| classify(path, source))?;
    tx.pragma_update(None, "user_version", CURRENT_VERSION)
        .map_err(|source| classify(path, source))?;
    tx.commit().map_err(|source| classify(path, source))?;
    Ok(())
}

pub(super) struct TaskRow {
    pub id: String,
    pub person: String,
    pub device: String,
    pub kind: String,
    pub target: String,
    pub state: String,
    pub progress: String,
    pub retry_count: i64,
    pub retry_of: Option<String>,
    pub created_at: i64,
    pub partial_failures: String,
    pub next_attempt_at: Option<i64>,
}

pub(super) struct SessionRow {
    pub id: String,
    pub person: String,
    pub device: String,
    pub item: String,
    pub state: String,
    pub position_ms: i64,
    pub plan: Option<String>,
    pub stop_reason: Option<String>,
}

pub(super) struct QueueRow {
    pub revision: i64,
    pub entries: String,
    pub cursor: Option<i64>,
    pub mode: String,
    pub sleep_at_ms: Option<i64>,
}

pub(super) fn row_to_task(row: &Row<'_>) -> rusqlite::Result<TaskRow> {
    Ok(TaskRow {
        id: row.get(0)?,
        person: row.get(1)?,
        device: row.get(2)?,
        kind: row.get(3)?,
        target: row.get(4)?,
        state: row.get(5)?,
        progress: row.get(6)?,
        retry_count: row.get(7)?,
        retry_of: row.get(8)?,
        created_at: row.get(9)?,
        partial_failures: row.get(10)?,
        next_attempt_at: row.get(11)?,
    })
}

pub(super) fn session_row(row: &Row<'_>) -> rusqlite::Result<SessionRow> {
    Ok(SessionRow {
        id: row.get(0)?,
        person: row.get(1)?,
        device: row.get(2)?,
        item: row.get(3)?,
        state: row.get(4)?,
        position_ms: row.get(5)?,
        plan: row.get(6)?,
        stop_reason: row.get(7)?,
    })
}

/// 会话行连同写版本（第 9 列）。
pub(super) fn session_and_version(row: &Row<'_>) -> rusqlite::Result<(SessionRow, u64)> {
    Ok((session_row(row)?, row.get::<_, i64>(8)? as u64))
}

pub(super) fn queue_row(row: &Row<'_>) -> rusqlite::Result<QueueRow> {
    Ok(QueueRow {
        revision: row.get(0)?,
        entries: row.get(1)?,
        cursor: row.get(2)?,
        mode: row.get(3)?,
        sleep_at_ms: row.get(4)?,
    })
}

/// 不是 SQLite 文件的错误单独成一类，消息里才能说清是哪个文件坏了。头合法、内容坏的文件仍归
/// `Database`，SQLite 报的错会原样带出来。
pub(super) fn classify(path: &Path, source: rusqlite::Error) -> MediaStoreError {
    if matches!(&source, rusqlite::Error::SqliteFailure(code, _) if code.code == rusqlite::ErrorCode::NotADatabase)
    {
        return MediaStoreError::Corrupt {
            path: path.to_owned(),
            source,
        };
    }
    MediaStoreError::Database {
        path: path.to_owned(),
        source,
    }
}
