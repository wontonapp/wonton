//! 发给设备的命令（ADR-0113 第 5 节“到设备的命令通道”）。`seq` 每台设备单调，记在 `command_seq`，
//! 命令清掉了也不回退：设备按“回执到哪个 `seq`”拉，回退了会把新命令当成已回执的跳过。

use rusqlite::{OptionalExtension, params};
use wonton_protocol::media::{DeviceCommand, MediaAction, TaskId, new_task_id};

use super::{MediaStoreError, Store};

/// 一条回执落到库里是什么情形。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AckState {
    /// 第一次回执，记上了。
    First,
    /// 回执过了，或者命令已经撤掉、清掉。
    Repeat,
    OtherDevice,
    SeqMismatch {
        expected: u64,
    },
}

impl Store {
    /// 给这台设备排一条命令，回带好 `command_id`、`seq` 的命令。
    pub fn push_command(
        &self,
        device: &str,
        body: &MediaAction,
    ) -> Result<DeviceCommand, MediaStoreError> {
        let body_json = self.json("body", body)?;
        let mut conn = self.conn()?;
        let tx = conn.transaction().map_err(|source| self.db_error(source))?;
        let seq: i64 = tx
            .query_row(
                "INSERT INTO command_seq (device, last) VALUES (?, 1)
                 ON CONFLICT(device) DO UPDATE SET last = last + 1
                 RETURNING last",
                params![device],
                |row| row.get(0),
            )
            .map_err(|source| self.db_error(source))?;
        let command = DeviceCommand {
            command_id: new_task_id(),
            seq: seq as u64,
            body: body.clone(),
        };
        tx.execute(
            "INSERT INTO commands (command_id, device, seq, body, acked) VALUES (?, ?, ?, ?, 0)",
            params![command.command_id.to_string(), device, seq, body_json],
        )
        .map_err(|source| self.db_error(source))?;
        tx.commit().map_err(|source| self.db_error(source))?;
        Ok(command)
    }

    /// 这台设备 `seq` 在 `after` 之后、还没回执的命令，按 `seq` 排。`after` 是设备说回执到的地方，
    /// 那以前的顺手清掉。
    pub fn commands_after(
        &self,
        device: &str,
        after: u64,
    ) -> Result<Vec<DeviceCommand>, MediaStoreError> {
        let after = i64::try_from(after).unwrap_or(i64::MAX);
        let conn = self.conn()?;
        conn.execute(
            "DELETE FROM commands WHERE device = ? AND seq <= ?",
            params![device, after],
        )
        .map_err(|source| self.db_error(source))?;
        let mut stmt = conn
            .prepare(
                "SELECT command_id, seq, body FROM commands
                 WHERE device = ? AND seq > ? AND acked = 0 ORDER BY seq",
            )
            .map_err(|source| self.db_error(source))?;
        let rows = stmt
            .query_map(params![device, after], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(|source| self.db_error(source))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|source| self.db_error(source))?;
        rows.into_iter()
            .map(|(id, seq, body)| {
                Ok(DeviceCommand {
                    command_id: self.parse_id(&id)?,
                    seq: seq as u64,
                    body: self.read_json("body", &body)?,
                })
            })
            .collect()
    }

    /// 记下这台设备对 `command` 的回执；只有设备和 `seq` 都对得上的第一次算数。
    pub fn ack_command(
        &self,
        device: &str,
        command: TaskId,
        seq: u64,
    ) -> Result<AckState, MediaStoreError> {
        let id = command.to_string();
        let conn = self.conn()?;
        let changed = conn
            .execute(
                "UPDATE commands SET acked = 1
                 WHERE command_id = ? AND device = ? AND seq = ? AND acked = 0",
                params![id, device, seq as i64],
            )
            .map_err(|source| self.db_error(source))?;
        if changed == 1 {
            return Ok(AckState::First);
        }
        let row: Option<(String, i64)> = conn
            .query_row(
                "SELECT device, seq FROM commands WHERE command_id = ?",
                params![id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|source| self.db_error(source))?;
        Ok(match row {
            Some((owner, _)) if owner != device => AckState::OtherDevice,
            Some((_, expected)) if expected as u64 != seq => AckState::SeqMismatch {
                expected: expected as u64,
            },
            _ => AckState::Repeat,
        })
    }

    /// 撤掉还没回执的命令（等回执超时）；回执过的留着，等设备拉的时候清。
    pub fn withdraw_command(&self, command: TaskId) -> Result<(), MediaStoreError> {
        let conn = self.conn()?;
        conn.execute(
            "DELETE FROM commands WHERE command_id = ? AND acked = 0",
            params![command.to_string()],
        )
        .map_err(|source| self.db_error(source))?;
        Ok(())
    }
}
