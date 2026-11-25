//! 播放会话的读写、活动会话与比较写。
//!
//! 会话每次写都加 `version`；停了的会话带 `stop_reason`。`(person) WHERE stop_reason IS NULL` 上有一条
//! 唯一索引，一人同时只能一条活动会话。

use rusqlite::{Connection, OptionalExtension, params};
use wonton_protocol::media::{
    MediaItemRef, PlaybackPlan, PlaybackSession, PlaybackSessionId, StopReason,
};

use super::MediaStoreError;
use super::Store;
use super::schema::{SessionRow, session_and_version, session_row};

/// 一条会话连同它的写版本；比较写靠这个版本，读的人不用看见它。
#[derive(Debug)]
pub struct StoredSession {
    pub session: PlaybackSession,
    pub version: u64,
}

impl Store {
    /// 存一条会话。同一个人最多一条停因为空的活动会话，撞上回 [`MediaStoreError::ActiveSessionExists`]。
    pub fn create_session(&self, session: &PlaybackSession) -> Result<(), MediaStoreError> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO sessions (id, person, device, item, state, state_tag, position_ms, plan,
                                   stop_reason)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                session.id.to_string(),
                session.person,
                session.device,
                self.json("item", &session.item)?,
                self.json("state", &session.state)?,
                session.state.as_str(),
                session.position_ms,
                self.session_plan(session)?,
                session.stop_reason.as_ref().map(stop_reason_raw),
            ],
        )
        .map_err(|source| match &source {
            rusqlite::Error::SqliteFailure(code, _)
                if code.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE =>
            {
                MediaStoreError::ActiveSessionExists {
                    path: self.path.clone(),
                    person: session.person.clone(),
                }
            }
            _ => self.db_error(source),
        })?;
        Ok(())
    }

    /// 列会话，按建立的先后；`person` 为空列所有人的。
    pub fn sessions(&self, person: Option<&str>) -> Result<Vec<PlaybackSession>, MediaStoreError> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, person, device, item, state, position_ms, plan, stop_reason
                 FROM sessions WHERE ? IS NULL OR person = ? ORDER BY rowid",
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

    /// 一条会话连同库里的写版本，服务层靠版本做比较写。
    pub fn stored_session(&self, id: PlaybackSessionId) -> Result<StoredSession, MediaStoreError> {
        let conn = self.conn()?;
        match self.stored_session_row(&conn, id)? {
            Some((row, version)) => Ok(StoredSession {
                session: self.assemble_session(row)?,
                version,
            }),
            None => Err(MediaStoreError::SessionNotFound {
                path: self.path.clone(),
                session: id,
            }),
        }
    }

    /// 这个人此刻的活动会话（停因为空的那条）。
    pub fn active_session(&self, person: &str) -> Result<Option<StoredSession>, MediaStoreError> {
        let conn = self.conn()?;
        let row: Option<(SessionRow, u64)> = conn
            .query_row(
                "SELECT id, person, device, item, state, position_ms, plan, stop_reason, version
                 FROM sessions WHERE person = ? AND stop_reason IS NULL
                 ORDER BY rowid DESC LIMIT 1",
                params![person],
                session_and_version,
            )
            .optional()
            .map_err(|source| self.db_error(source))?;
        row.map(|(row, version)| {
            Ok(StoredSession {
                session: self.assemble_session(row)?,
                version,
            })
        })
        .transpose()
    }

    /// 带着读到的版本写整条会话，写成功版本加一；落 0 行说明被别处改了。
    pub fn save_session_cas(
        &self,
        session: &PlaybackSession,
        expected_version: u64,
    ) -> Result<bool, MediaStoreError> {
        let conn = self.conn()?;
        let changed = conn
            .execute(
                "UPDATE sessions SET device = ?, item = ?, state = ?, state_tag = ?, position_ms = ?,
                                     plan = ?, stop_reason = ?, version = version + 1
                 WHERE id = ? AND person = ? AND version = ?",
                params![
                    session.device,
                    self.json("item", &session.item)?,
                    self.json("state", &session.state)?,
                    session.state.as_str(),
                    session.position_ms,
                    self.session_plan(session)?,
                    session.stop_reason.as_ref().map(stop_reason_raw),
                    session.id.to_string(),
                    session.person,
                    expected_version,
                ],
            )
            .map_err(|source| self.db_error(source))?;
        Ok(changed > 0)
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
            stop_reason: row.stop_reason.as_deref().map(stop_reason_from_raw),
        })
    }

    fn session_plan(&self, session: &PlaybackSession) -> Result<Option<String>, MediaStoreError> {
        session
            .plan
            .as_ref()
            .map(|plan| self.json("plan", plan))
            .transpose()
    }

    fn stored_session_row(
        &self,
        conn: &Connection,
        id: PlaybackSessionId,
    ) -> Result<Option<(SessionRow, u64)>, MediaStoreError> {
        conn.query_row(
            "SELECT id, person, device, item, state, position_ms, plan, stop_reason, version
             FROM sessions WHERE id = ?",
            params![id.to_string()],
            session_and_version,
        )
        .optional()
        .map_err(|source| self.db_error(source))
    }
}

/// 停因按原值存（没有载荷）。
fn stop_reason_raw(reason: &StopReason) -> &str {
    match reason {
        StopReason::User => "user",
        StopReason::Agent => "agent",
        StopReason::TakenOver => "taken_over",
        StopReason::Finished => "finished",
        StopReason::Error => "error",
        StopReason::Other(raw) => raw,
    }
}

fn stop_reason_from_raw(raw: &str) -> StopReason {
    match raw {
        "user" => StopReason::User,
        "agent" => StopReason::Agent,
        "taken_over" => StopReason::TakenOver,
        "finished" => StopReason::Finished,
        "error" => StopReason::Error,
        _ => StopReason::Other(raw.to_owned()),
    }
}
