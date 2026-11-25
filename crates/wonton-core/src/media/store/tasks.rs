//! 任务的读写与逐项条目。
//!
//! `save_task` 只写任务行，条目由 `create_task`、`enqueue_items`、`save_item` 单独写。

use rusqlite::types::Value;
use rusqlite::{Connection, OptionalExtension, params, params_from_iter};
use wonton_protocol::media::{MediaTask, TaskId, TaskKind, TaskState, TaskTarget, UploadItem};

use super::MediaStoreError;
use super::Store;
use super::schema::{TaskRow, row_to_task};

impl Store {
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
                                retry_count, retry_of, created_at, partial_failures, next_attempt_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
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
                task.next_attempt_at,
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
                    created_at, partial_failures, next_attempt_at FROM tasks",
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

    /// 回写任务的状态、进度等普通字段；只有库里的状态仍是 `expected`、`retry_count` 仍是
    /// `expected_retry_count` 才写，返回是否落上。
    ///
    /// 比较条件交给调用方：读到什么就原样传什么，落不上说明被别处改了，重读再来。带上计数是为了两次
    /// 同时到达、都会计数的失败不会只记一次、也不会互相顶掉 `next_attempt_at`（`rate_limited` 这种不计数的不受保护）。
    /// **不写逐项条目**：`target` 里带的 `UploadItem` 会被忽略，条目只认 `create_task`、`enqueue_items`、
    /// `save_item` 写的行。改了某一项要另外调 [`Self::save_item`]。
    /// `next_attempt_at` 也不校验状态，由服务层维持“只在 `WaitingNetwork` 时有值”。
    pub fn save_task(
        &self,
        task: &MediaTask,
        expected: &TaskState,
        expected_retry_count: u8,
    ) -> Result<bool, MediaStoreError> {
        let conn = self.conn()?;
        let target = match &task.target {
            TaskTarget::Upload(_) => TaskTarget::Upload(Vec::new()),
            other => other.clone(),
        };
        let changed = conn
            .execute(
                "UPDATE tasks SET device = ?, kind = ?, target = ?, state = ?, state_tag = ?,
                                  progress = ?, retry_count = ?, retry_of = ?, created_at = ?,
                                  partial_failures = ?, next_attempt_at = ?
                 WHERE id = ? AND person = ? AND state_tag = ? AND retry_count = ?",
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
                    task.next_attempt_at,
                    task.id.to_string(),
                    task.person,
                    expected.as_str(),
                    expected_retry_count,
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
                    created_at, partial_failures, next_attempt_at FROM tasks WHERE id = ? AND person = ?",
            params![id.to_string(), person],
            row_to_task,
        )
        .optional()
        .map_err(|source| self.db_error(source))
    }

    fn task_any_row(
        &self,
        conn: &Connection,
        id: TaskId,
    ) -> Result<Option<TaskRow>, MediaStoreError> {
        conn.query_row(
            "SELECT id, person, device, kind, target, state, progress, retry_count, retry_of,
                    created_at, partial_failures, next_attempt_at FROM tasks WHERE id = ?",
            params![id.to_string()],
            row_to_task,
        )
        .optional()
        .map_err(|source| self.db_error(source))
    }

    /// 按编号读任务，不按人分；没有报 `TaskNotFound`。服务层手里只有任务号时用它。
    pub fn task_any(&self, id: TaskId) -> Result<MediaTask, MediaStoreError> {
        let conn = self.conn()?;
        match self.task_any_row(&conn, id)? {
            Some(row) => self.assemble_task(&conn, row),
            None => Err(MediaStoreError::TaskNotFound {
                path: self.path.clone(),
                task: id,
            }),
        }
    }

    /// 等网络、到点该重试的任务，按到点先后。
    pub fn due_tasks(&self, at: u64) -> Result<Vec<MediaTask>, MediaStoreError> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, person, device, kind, target, state, progress, retry_count, retry_of,
                        created_at, partial_failures, next_attempt_at FROM tasks
                 WHERE state_tag = ? AND next_attempt_at <= ? ORDER BY next_attempt_at, id",
            )
            .map_err(|source| self.db_error(source))?;
        let rows: Vec<TaskRow> = stmt
            .query_map(
                params![TaskState::WaitingNetwork.as_str(), at as i64],
                row_to_task,
            )
            .map_err(|source| self.db_error(source))?
            .collect::<Result<_, _>>()
            .map_err(|source| self.db_error(source))?;
        rows.into_iter()
            .map(|row| self.assemble_task(&conn, row))
            .collect()
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
            // 上限远到不了；真被外部写大了就夹住，不回转。
            retry_count: u8::try_from(row.retry_count).unwrap_or(u8::MAX),
            retry_of: row
                .retry_of
                .as_deref()
                .map(|raw| self.parse_id(raw))
                .transpose()?,
            created_at: row.created_at as u64,
            next_attempt_at: row.next_attempt_at.map(|value| value as u64),
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
