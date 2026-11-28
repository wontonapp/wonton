//! 媒体库的建表与升级语句、行到结构体的搬运。
//!
//! 结构版本记在 `PRAGMA user_version`：`MIGRATIONS` 从旧到新一版一条，`open` 时按库里记的版本往后补，
//! 每条迁移连同版本号写在一个事务里。加迁移时同时把 `CURRENT_VERSION` 加一。

use std::path::Path;

use rusqlite::Row;

use super::MediaStoreError;

pub(super) const CURRENT_VERSION: i64 = 5;

/// 第 1 版：最初的五张表。逐项条目单独成表，偏移一次只改一行。
pub(crate) const SCHEMA_V1: &str = "
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
    partial_failures TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS tasks_by_person ON tasks(person, created_at);
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
    -- 和 state 一起写；M9 靠它找这个人的活动会话
    state_tag TEXT NOT NULL,
    position_ms INTEGER NOT NULL,
    plan TEXT
);
CREATE INDEX IF NOT EXISTS sessions_by_person ON sessions(person);
CREATE TABLE IF NOT EXISTS queue (
    person TEXT PRIMARY KEY,
    revision INTEGER NOT NULL,
    entries TEXT NOT NULL,
    cursor INTEGER,
    mode TEXT NOT NULL,
    sleep_at_ms INTEGER
);
CREATE TABLE IF NOT EXISTS quota (
    person TEXT NOT NULL,
    ledger TEXT NOT NULL,
    device TEXT NOT NULL,
    used INTEGER NOT NULL,
    cap INTEGER,
    PRIMARY KEY (person, ledger, device)
);
";

/// 第 2 版：上传配额按提供方分开记（ADR-0113 第 3 节），加 `provider` 列。主键变了，只能重建表。
///
/// 第 1 版的账没有提供方。离线的账本来就按设备分，原样搬（provider 留空串）；上传的账每人并成
/// provider 与 device 都是空串的一行，等这个人的第一笔上传落到具体提供方时接手它（`store.rs`
/// 的 `record_usage`）。并成一行是为了让接手的查询只有一个落点。
pub(super) const MIGRATE_V2: &str = "
CREATE TABLE quota_v2 (
    person TEXT NOT NULL,
    ledger TEXT NOT NULL,
    provider TEXT NOT NULL,
    device TEXT NOT NULL,
    used INTEGER NOT NULL,
    cap INTEGER,
    PRIMARY KEY (person, ledger, provider, device)
);
INSERT INTO quota_v2 (person, ledger, provider, device, used, cap)
    SELECT person, ledger, '', device, used, cap FROM quota WHERE ledger = 'device_offline';
INSERT INTO quota_v2 (person, ledger, provider, device, used, cap)
    SELECT person, ledger, '', '', SUM(used),
           CASE WHEN COUNT(cap) < COUNT(*) THEN NULL ELSE SUM(cap) END
    FROM quota WHERE ledger = 'provider_upload' GROUP BY person, ledger;
DROP TABLE quota;
ALTER TABLE quota_v2 RENAME TO quota;
";

/// 第 3 版：任务的下一次自动重试时间（M8）；等待网络的任务按它找。
pub(super) const MIGRATE_V3: &str = "
ALTER TABLE tasks ADD COLUMN next_attempt_at INTEGER;
CREATE INDEX IF NOT EXISTS tasks_due ON tasks(state_tag, next_attempt_at);
";

/// 第 4 版：会话记停因，写每次加版本号；一人同时只能有一条活动会话（停因为空的）。
/// 建索引前把同一个人多出来的活动会话按旧到新置成接管终态，留 rowid 最大的一条。
pub(super) const MIGRATE_V4: &str = "
ALTER TABLE sessions ADD COLUMN stop_reason TEXT;
ALTER TABLE sessions ADD COLUMN version INTEGER NOT NULL DEFAULT 0;
UPDATE sessions
SET state = '\"cancelled\"', state_tag = 'cancelled', stop_reason = 'taken_over'
WHERE stop_reason IS NULL
  AND rowid NOT IN (SELECT max(rowid) FROM sessions WHERE stop_reason IS NULL GROUP BY person);
CREATE UNIQUE INDEX sessions_one_active ON sessions(person) WHERE stop_reason IS NULL;
";

/// 第 5 版：任务和会话记下提供方，会话记下起播时的设备档；上传项在提供方那边的号单独一张表
/// （ADR-0113 第 2 节“字节路径”）。
pub(super) const MIGRATE_V5: &str = "
ALTER TABLE tasks ADD COLUMN provider TEXT;
ALTER TABLE sessions ADD COLUMN provider TEXT;
ALTER TABLE sessions ADD COLUMN profile TEXT;
CREATE TABLE uploads (
    person TEXT NOT NULL,
    task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    item_id TEXT NOT NULL,
    provider TEXT NOT NULL,
    upload_id TEXT NOT NULL,
    PRIMARY KEY (task_id, item_id)
);
";

pub(super) const MIGRATIONS: &[(i64, &str)] = &[
    (1, SCHEMA_V1),
    (2, MIGRATE_V2),
    (3, MIGRATE_V3),
    (4, MIGRATE_V4),
    (5, MIGRATE_V5),
];

/// 把库补到 [`CURRENT_VERSION`]；比核心新的库拒绝打开，免得更新的核心写坏后被旧版改。
pub(super) fn migrate(conn: &rusqlite::Connection, path: &Path) -> Result<(), MediaStoreError> {
    let found: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|source| classify(path, source))?;
    if found > CURRENT_VERSION {
        return Err(MediaStoreError::TooNew {
            path: path.to_owned(),
            found,
            supported: CURRENT_VERSION,
        });
    }
    for (version, sql) in MIGRATIONS.iter().filter(|(version, _)| *version > found) {
        let tx = conn
            .unchecked_transaction()
            .map_err(|source| classify(path, source))?;
        tx.execute_batch(sql)
            .map_err(|source| classify(path, source))?;
        tx.pragma_update(None, "user_version", version)
            .map_err(|source| classify(path, source))?;
        tx.commit().map_err(|source| classify(path, source))?;
    }
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
