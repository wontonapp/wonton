//! 输入框里的草稿：只存本机，按房间分开，主人换房间、关掉 App 再回来还在。正在回复的消息和正文一起存
//! （ADR-0073 第 2 节）。
//!
//! 每敲一个字发一次，写进 matrix-sdk 的状态库（和 Element 一样用 `m.room.draft` 的那套存储）。写入排一个队，
//! 并发写会让先敲的内容后落盘。

use std::sync::{Arc, Weak};

use tokio::sync::{mpsc, oneshot};
use wonton_l10n::l10n;
use wonton_matrix::{Draft, Error, MatrixSession, OwnedEventId, OwnedRoomId};
use wonton_protocol::{ComposerDraft, CoreError, ReplyQuote};

use super::ChatLink;
use super::confirmation::core_account;
use super::links::LinkTargets;
use super::reply::quoted_message;
use super::send::reply_target;
use crate::l10n::{CATALOG, language};

/// 排队落盘的一笔草稿。
pub(super) enum DraftWrite {
    Save(OwnedRoomId, Draft),
    /// 已经发出去的正文：存着的草稿还是它才删，回复对象一起删（发送途中又打了字就不是它了）。
    Forget(OwnedRoomId, String),
    /// 用图片回复了这条：存着的草稿还在回复它的话，只去掉回复对象，正文留着。
    ForgetReply(OwnedRoomId, OwnedEventId),
    /// 读回草稿，发给 `ack`。排在前面的写入都落盘了才回。
    Read(OwnedRoomId, oneshot::Sender<Option<Draft>>),
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
            DraftWrite::Save(room_id, draft) => {
                if let Err(error) = save_draft(&session, &room_id, &draft).await {
                    chat.report_op_failure("chat-draft-save-failed", &error);
                }
            }
            DraftWrite::Forget(room_id, body) => {
                let Some(session) = &session else { continue };
                let Some(stored) = session.draft(&room_id).await.ok().flatten() else {
                    continue;
                };
                if stored.body.trim() != body.trim() {
                    continue;
                }
                let empty = Draft {
                    body: String::new(),
                    reply_to: None,
                };
                if let Err(error) = session.save_draft(&room_id, &empty).await {
                    chat.report_op_failure("chat-draft-clear-failed", &error);
                }
            }
            DraftWrite::ForgetReply(room_id, event_id) => {
                let Some(session) = &session else { continue };
                let Some(stored) = session.draft(&room_id).await.ok().flatten() else {
                    continue;
                };
                if stored.reply_to.as_ref() != Some(&event_id) {
                    continue;
                }
                let kept = Draft {
                    body: stored.body,
                    reply_to: None,
                };
                if let Err(error) = session.save_draft(&room_id, &kept).await {
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
    draft: &Draft,
) -> Result<(), Error> {
    match session {
        Some(session) => session.save_draft(room_id, draft).await,
        None => Err(Error::NotReady(l10n!(
            CATALOG,
            language(),
            "chat-draft-not-signed-in"
        ))),
    }
}

impl ChatLink {
    pub(super) fn set_draft(
        &self,
        room_id: &str,
        body: &str,
        reply_to: Option<&str>,
    ) -> Result<(), CoreError> {
        let reply_to = reply_target(reply_to)?;
        let (_, room_id) = self.target(room_id)?;
        let draft = Draft {
            body: body.to_owned(),
            reply_to,
        };
        self.enqueue_draft(DraftWrite::Save(room_id, draft));
        Ok(())
    }

    pub(super) fn enqueue_draft(&self, write: DraftWrite) {
        // 写入任务随这个发送端一起活着，排不进去只可能是核心正在销毁。
        let _ = self.draft_writes.send(write);
    }

    /// 房间输入框里的草稿，没打过字也没在回复时是 `None`。只在本机，换房间、重启核心都还在。回复对象那条消息本机
    /// 没有时向服务器取，取不到给 `Unavailable`（壳照样能拿它的事件 ID 发回复）。
    pub(crate) async fn draft(&self, room_id: &str) -> Result<Option<ComposerDraft>, CoreError> {
        let (session, room_id) = self.target(room_id)?;
        let (ack, received) = oneshot::channel();
        self.enqueue_draft(DraftWrite::Read(room_id.clone(), ack));
        let stored = received.await.map_err(|_| CoreError::NotReady {
            reason: l10n!(CATALOG, language(), "chat-draft-read-interrupted"),
        })?;
        let Some(stored) = stored else {
            return Ok(None);
        };
        let reply = match stored.reply_to {
            Some(event_id) => Some(match session.quoted_event(&room_id, &event_id).await {
                Ok(message) => ReplyQuote::Ready {
                    message: quoted_message(
                        event_id.to_string(),
                        message,
                        &core_account(session.user_id()),
                        &LinkTargets::of(&self.plugins.view()),
                    ),
                },
                Err(error) => {
                    tracing::info!(%event_id, "couldn't load the message the draft replies to: {error}");
                    ReplyQuote::Unavailable {
                        event_id: event_id.to_string(),
                    }
                }
            }),
            None => None,
        };
        Ok(Some(ComposerDraft {
            body: stored.body,
            reply,
        }))
    }
}
