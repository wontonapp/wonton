//! 核心发给主人的通知房间“Wonton”（ADR-0016）：核心账号和主人的私聊，确认卡片发在这里。核心没有端到端加密
//! 设备，这个房间不加密。
//!
//! 房间 ID 记在核心的状态目录里，核心和主人都还在房间里就一直沿用，否则新建一个。

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::{Value, json};
use tokio::sync::Mutex;

use super::api::MatrixError;
use super::{Matrix, Session};

/// 通知房间的名字。
const ROOM_NAME: &str = "Wonton";

pub struct NoticeRoom {
    matrix: Arc<Matrix>,
    /// 主人的用户名（localpart）。
    owner: String,
    /// 记房间 ID 的文件。
    saved: PathBuf,
    room: Mutex<Option<String>>,
}

impl NoticeRoom {
    pub fn new(matrix: Arc<Matrix>, owner: &str, saved: PathBuf) -> Self {
        Self {
            matrix,
            owner: owner.to_owned(),
            saved,
            room: Mutex::new(None),
        }
    }

    /// 以核心账号发一条消息，返回事件 ID。
    pub async fn send(&self, content: &Value) -> Result<String, MatrixError> {
        let room = self.room().await?;
        self.matrix
            .core_session()
            .send_message(&room, content)
            .await
    }

    /// 以核心账号把 `event` 编辑成 `content`（`m.replace`）。
    pub async fn edit(&self, event: &str, content: &Value) -> Result<(), MatrixError> {
        let room = self.room().await?;
        let mut edit = content.clone();
        if let Some(body) = content["body"].as_str() {
            edit["body"] = json!(format!("* {body}"));
        }
        edit["m.new_content"] = content.clone();
        edit["m.relates_to"] = json!({"rel_type": "m.replace", "event_id": event});
        self.matrix
            .core_session()
            .send_message(&room, &edit)
            .await
            .map(|_| ())
    }

    /// 通知房间的 ID：记下的还能用就沿用，否则新建。
    async fn room(&self) -> Result<String, MatrixError> {
        let mut room = self.room.lock().await;
        if let Some(id) = room.as_ref() {
            return Ok(id.clone());
        }
        let core = self.matrix.core_session();
        let owner = self.matrix.user_id(&self.owner);
        let saved = std::fs::read_to_string(&self.saved)
            .ok()
            .map(|s| s.trim().to_owned());
        let id = match saved {
            Some(id) if !id.is_empty() && both_joined(&core, &id, &owner).await? => id,
            _ => self.create(&core, &owner).await?,
        };
        *room = Some(id.clone());
        Ok(id)
    }

    async fn create(&self, core: &Session<'_>, owner: &str) -> Result<String, MatrixError> {
        let id = core.create_notice_room(ROOM_NAME, owner).await?;
        let handed_over = async {
            core.add_direct(owner, &id).await?;
            self.matrix
                .owner_joins(&self.owner, core.user_id(), &id)
                .await
        }
        .await;
        if let Err(e) = handed_over {
            if let Err(cleanup) = core.abandon(&id, owner).await {
                tracing::error!(room = %id, error = %cleanup, "撤回没建好的通知房间失败");
            }
            return Err(e);
        }
        std::fs::write(&self.saved, &id)
            .map_err(|e| MatrixError::Invalid(format!("记下通知房间失败：{e}")))?;
        tracing::info!(room = %id, "建好了和主人的通知房间");
        Ok(id)
    }
}

/// 核心和主人是不是都还在房间里。核心已经不在房间里时 Tuwunel 回 `M_FORBIDDEN`。
async fn both_joined(core: &Session<'_>, room: &str, owner: &str) -> Result<bool, MatrixError> {
    match core.joined_members(room).await {
        Ok(members) => Ok(members.iter().any(|m| m == owner)),
        Err(e) if matches!(e.errcode(), "M_FORBIDDEN" | "M_NOT_FOUND") => Ok(false),
        Err(e) => Err(e),
    }
}
