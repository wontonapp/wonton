//! 输入框里的草稿：只存本机，按房间分开，主人换房间、关掉 App 再回来还在。
//!
//! 每敲一个字发一次，写进 matrix-sdk 的状态库（和 Element 一样用 `m.room.draft` 的那套存储）。写入排一个队，
//! 并发写会让先敲的内容后落盘。

use std::sync::{Arc, Weak};

use tokio::sync::{mpsc, oneshot};
use wonton_l10n::l10n;
use wonton_matrix::{Error, MatrixSession, OwnedRoomId};
use wonton_protocol::CoreError;

use super::ChatLink;
use crate::l10n::{CATALOG, language};

/// 排队落盘的一笔草稿。
pub(super) enum DraftWrite {
    Save(OwnedRoomId, String),
    /// 已经发出去的正文：存着的草稿还是它才删（发送途中又打了字就不是它了）。
    Forget(OwnedRoomId, String),
    /// 读回草稿，发给 `ack`。排在前面的写入都落盘了才回。
    Read(OwnedRoomId, oneshot::Sender<Option<String>>),
}

/// 按排队的顺序写草稿。会话没了就结束。
///
/// 写的是本机状态库的一行，快了就当写完；写不进去放进 `problem`，主人看得到草稿没存上。
pub(super) async fn write_drafts(
    chat: Weak<ChatLink>,
    mut writes: mpsc::UnboundedReceiver<DraftWrite>,
) {
    while let Some(write) = writes.recv().await {
        let Some(chat) = chat.upgrade() else {
            break;
        };
        let session = chat.current().ok();
        match write {
            DraftWrite::Save(room_id, body) => {
                if let Err(error) = save_draft(&session, &room_id, &body).await {
                    chat.report_op_failure("chat-draft-save-failed", &error);
                }
            }
            DraftWrite::Forget(room_id, body) => {
                let Some(session) = &session else { continue };
                let Some(stored) = session.draft(&room_id).await.ok().flatten() else {
                    continue;
                };
                if stored.trim() != body.trim() {
                    continue;
                }
                if let Err(error) = session.save_draft(&room_id, "").await {
                    chat.report_op_failure("chat-draft-clear-failed", &error);
                }
            }
            DraftWrite::Read(room_id, ack) => {
                let draft = match &session {
                    Some(session) => session.draft(&room_id).await.ok().flatten(),
                    None => None,
                };
                let _ = ack.send(draft);
            }
        }
    }
}

async fn save_draft(
    session: &Option<Arc<MatrixSession>>,
    room_id: &OwnedRoomId,
    body: &str,
) -> Result<(), Error> {
    match session {
        Some(session) => session.save_draft(room_id, body).await,
        None => Err(Error::NotReady(l10n!(
            CATALOG,
            language(),
            "chat-draft-not-signed-in"
        ))),
    }
}

impl ChatLink {
    pub(super) fn set_draft(&self, room_id: &str, body: &str) -> Result<(), CoreError> {
        let (_, room_id) = self.target(room_id)?;
        self.enqueue_draft(DraftWrite::Save(room_id, body.to_owned()));
        Ok(())
    }

    pub(super) fn enqueue_draft(&self, write: DraftWrite) {
        // 写入任务随这个发送端一起活着，排不进去只可能是核心正在销毁。
        let _ = self.draft_writes.send(write);
    }

    /// 房间输入框里的草稿，没打过字时是 `None`。只在本机，换房间、重启核心都还在。
    pub(crate) async fn draft(&self, room_id: &str) -> Result<Option<String>, CoreError> {
        let (_, room_id) = self.target(room_id)?;
        let (ack, received) = oneshot::channel();
        self.enqueue_draft(DraftWrite::Read(room_id, ack));
        received.await.map_err(|_| CoreError::NotReady {
            reason: l10n!(CATALOG, language(), "chat-draft-read-interrupted"),
        })
    }
}
