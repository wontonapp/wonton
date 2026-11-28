//! 任务与会话记下的提供方，和上传项在提供方那边的上传号（ADR-0113 第 2 节“字节路径”）。
//!
//! 提供方一记下就不再换：续写、进度、停止都发给同一家；`upload_id` 只存在核心，客户端不记。

use rusqlite::{OptionalExtension, params};
use wonton_protocol::media::{DeviceProfile, PlaybackSessionId, TaskId};

use super::{MediaStoreError, Store};

/// 一项上传在提供方那边的号。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadBinding {
    pub provider: String,
    pub upload_id: String,
}

impl Store {
    /// 任务记下的提供方。
    pub fn task_provider(
        &self,
        person: &str,
        task: TaskId,
    ) -> Result<Option<String>, MediaStoreError> {
        let conn = self.conn()?;
        let row: Option<Option<String>> = conn
            .query_row(
                "SELECT provider FROM tasks WHERE id = ? AND person = ?",
                params![task.to_string(), person],
                |row| row.get(0),
            )
            .optional()
            .map_err(|source| self.db_error(source))?;
        Ok(row.flatten())
    }

    /// 给还没记提供方的任务记下一个，返回最后记着的那个（别处先记了就是别处的）。
    pub fn bind_task_provider(
        &self,
        person: &str,
        task: TaskId,
        provider: &str,
    ) -> Result<String, MediaStoreError> {
        let conn = self.conn()?;
        conn.execute(
            "UPDATE tasks SET provider = ? WHERE id = ? AND person = ? AND provider IS NULL",
            params![provider, task.to_string(), person],
        )
        .map_err(|source| self.db_error(source))?;
        drop(conn);
        self.task_provider(person, task)?
            .ok_or_else(|| MediaStoreError::TaskNotFound {
                path: self.path.clone(),
                task,
            })
    }

    pub fn session_provider(
        &self,
        session: PlaybackSessionId,
    ) -> Result<Option<String>, MediaStoreError> {
        let conn = self.conn()?;
        let row: Option<Option<String>> = conn
            .query_row(
                "SELECT provider FROM sessions WHERE id = ?",
                params![session.to_string()],
                |row| row.get(0),
            )
            .optional()
            .map_err(|source| self.db_error(source))?;
        Ok(row.flatten())
    }

    /// 起播时记下提供方和设备档：切轨、换曲时按同一个档重取计划。
    pub fn bind_session_route(
        &self,
        session: PlaybackSessionId,
        provider: &str,
        profile: &DeviceProfile,
    ) -> Result<(), MediaStoreError> {
        let conn = self.conn()?;
        let changed = conn
            .execute(
                "UPDATE sessions SET provider = ?, profile = ? WHERE id = ?",
                params![
                    provider,
                    self.json("profile", profile)?,
                    session.to_string()
                ],
            )
            .map_err(|source| self.db_error(source))?;
        if changed == 0 {
            return Err(MediaStoreError::SessionNotFound {
                path: self.path.clone(),
                session,
            });
        }
        Ok(())
    }

    pub fn session_profile(
        &self,
        session: PlaybackSessionId,
    ) -> Result<Option<DeviceProfile>, MediaStoreError> {
        let conn = self.conn()?;
        let row: Option<Option<String>> = conn
            .query_row(
                "SELECT profile FROM sessions WHERE id = ?",
                params![session.to_string()],
                |row| row.get(0),
            )
            .optional()
            .map_err(|source| self.db_error(source))?;
        row.flatten()
            .map(|text| self.read_json("profile", &text))
            .transpose()
    }

    pub fn upload_binding(
        &self,
        person: &str,
        task: TaskId,
        item: &str,
    ) -> Result<Option<UploadBinding>, MediaStoreError> {
        let conn = self.conn()?;
        conn.query_row(
            "SELECT provider, upload_id FROM uploads WHERE person = ? AND task_id = ? AND item_id = ?",
            params![person, task.to_string(), item],
            |row| {
                Ok(UploadBinding {
                    provider: row.get(0)?,
                    upload_id: row.get(1)?,
                })
            },
        )
        .optional()
        .map_err(|source| self.db_error(source))
    }

    /// 记下（或换掉）一项的上传号；提供方说不认识旧号、重开一个时用它换。
    pub fn bind_upload(
        &self,
        person: &str,
        task: TaskId,
        item: &str,
        binding: &UploadBinding,
    ) -> Result<(), MediaStoreError> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO uploads (person, task_id, item_id, provider, upload_id) VALUES (?, ?, ?, ?, ?)
             ON CONFLICT(task_id, item_id) DO UPDATE SET
                 provider = excluded.provider, upload_id = excluded.upload_id",
            params![person, task.to_string(), item, binding.provider, binding.upload_id],
        )
        .map_err(|source| self.db_error(source))?;
        Ok(())
    }
}
