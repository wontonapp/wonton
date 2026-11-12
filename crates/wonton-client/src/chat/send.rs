//! 发文字和图片，可以是对某条消息的回复（ADR-0073 第 2 节：回复的是哪条由壳每次显式给出）。

use std::sync::Arc;

use wonton_l10n::l10n;
use wonton_matrix::OwnedEventId;
use wonton_protocol::{ChatAccount, CoreError, ImageUpload};

use super::draft::DraftWrite;
use super::view::core_error;
use super::{ChatLink, invalid, onboarding};
use crate::l10n::{CATALOG, language};

/// 壳给的被回复消息的事件 ID。
pub(super) fn reply_target(reply_to: Option<&str>) -> Result<Option<OwnedEventId>, CoreError> {
    reply_to
        .map(|id| {
            id.parse::<OwnedEventId>().map_err(|e| {
                invalid(&l10n!(
                    CATALOG,
                    language(),
                    "chat-invalid-event-id",
                    error = e.to_string()
                ))
            })
        })
        .transpose()
}

impl ChatLink {
    pub(super) fn send_text(
        self: &Arc<Self>,
        room_id: &str,
        body: &str,
        reply_to: Option<&str>,
    ) -> Result<(), CoreError> {
        let body = body.trim().to_owned();
        if body.is_empty() {
            return Err(invalid(&l10n!(CATALOG, language(), "chat-empty-message")));
        }
        let reply_to = reply_target(reply_to)?;
        let chat = self.clone();
        let sent = self.sent.clone();
        let (_, room_id) = self.send_target(room_id)?;
        let sent_room = room_id.to_string();
        self.spawn_op_with_success(
            "chat-op-send-text-failed",
            async move {
                let room = chat.room(&room_id).await?;
                // 预览的图片传不上去时不挡消息：正文照发，原因报出来（取不到的类别发送框里已经显示过）。
                let extra = match chat.preview_extra(&room, &room_id, &body).await {
                    Ok(extra) => extra,
                    Err(error) => {
                        chat.report_op_failure("chat-op-send-preview-failed", &error);
                        None
                    }
                };
                match &reply_to {
                    Some(event_id) => match extra {
                        Some(extra) => room.send_reply_with_extra(&body, event_id, extra).await?,
                        None => room.send_reply(&body, event_id).await?,
                    },
                    None => match extra {
                        Some(extra) => room.send_text_with_extra(&body, extra).await?,
                        None => room.send_text(&body).await?,
                    },
                }
                // 发出去了，草稿（连同回复对象）留着没用了；发送途中又打的字不算。
                chat.enqueue_draft(DraftWrite::Forget(room_id.clone(), body));
                // 点掉的预览记录跟着作废：下次打同样的链接重新出预览。
                chat.previews.forget(&room_id);
                Ok(())
            },
            move || onboarding::record_sent(&sent, sent_room),
        );
        Ok(())
    }

    /// 冷启动先等会话和目标房间恢复；校验旁聊关系后交给持久待发队列，不改草稿。
    pub(crate) async fn prepare_push_reply(
        self: &Arc<Self>,
        room_id: &str,
        body: &str,
        reply_to: Option<&str>,
        thread_root: Option<&str>,
    ) -> Result<wonton_matrix::PreparedReply, CoreError> {
        let body = body.trim().to_owned();
        if body.is_empty() {
            return Err(invalid(&l10n!(CATALOG, language(), "chat-empty-message")));
        }
        let reply_to = reply_target(reply_to)?;
        let thread_root = reply_target(thread_root)?;
        let sending = async {
            let mut view = self.subscribe();
            while self.current().is_err() {
                if let super::ChatAccount::Failed { message, .. } =
                    &view.borrow_and_update().account
                {
                    return Err(CoreError::NotReady {
                        reason: message.clone(),
                    });
                }
                view.changed().await.map_err(|_| CoreError::NotReady {
                    reason: l10n!(CATALOG, language(), "chat-not-signed-in"),
                })?;
            }
            let (session, room_id) = self.send_target(room_id)?;
            session
                .prepare_reply(&room_id, &body, reply_to.as_deref(), thread_root.as_deref())
                .await
                .map_err(|error| self.convert_op_failure("chat-op-send-text-failed", &error))
        };
        let result = tokio::time::timeout(std::time::Duration::from_secs(30), sending)
            .await
            .unwrap_or_else(|_| {
                Err(CoreError::TransportUnavailable {
                    reason: l10n!(CATALOG, language(), "chat-send-no-response"),
                })
            });
        if let Err(error) = &result {
            self.view.send_modify(|view| {
                view.problem = Some(wonton_protocol::ChatProblem {
                    reason: match error {
                        CoreError::NotReady { .. } => wonton_protocol::ChatFailure::NotReady,
                        CoreError::InvalidInput { .. } => {
                            wonton_protocol::ChatFailure::InvalidInput
                        }
                        CoreError::Unauthorized { .. } => {
                            wonton_protocol::ChatFailure::Unauthorized
                        }
                        CoreError::TransportUnavailable { .. } => {
                            wonton_protocol::ChatFailure::TransportUnavailable
                        }
                        _ => wonton_protocol::ChatFailure::Upstream,
                    },
                    message: error.to_string(),
                })
            });
        }
        result
    }

    /// 操作失败：记进 `problem`（打开 App 看得到），同时把类别交回去给通知。
    fn convert_op_failure(
        &self,
        failure_id: &'static str,
        error: &wonton_matrix::Error,
    ) -> CoreError {
        self.report_op_failure(failure_id, error);
        core_error(error)
    }

    pub(super) fn send_image(
        self: &Arc<Self>,
        room_id: &str,
        image: ImageUpload,
        reply_to: Option<&str>,
    ) -> Result<(), CoreError> {
        if !image.mime_type.starts_with("image/") {
            return Err(invalid(&l10n!(
                CATALOG,
                language(),
                "chat-not-image-type",
                file_type = image.mime_type.as_str()
            )));
        }
        let reply_to = reply_target(reply_to)?;
        let chat = self.clone();
        let sent = self.sent.clone();
        let (_, room_id) = self.send_target(room_id)?;
        let sent_room = room_id.to_string();
        let upload = wonton_matrix::ImageUpload {
            filename: image.filename,
            mime_type: image.mime_type,
            data: image.data,
            width: image.width,
            height: image.height,
            thumbnail: image
                .thumbnail
                .map(|thumbnail| wonton_matrix::ImageThumbnail {
                    mime_type: thumbnail.mime_type,
                    data: thumbnail.data,
                    width: thumbnail.width,
                    height: thumbnail.height,
                }),
        };
        self.spawn_op_with_success(
            "chat-op-send-image-failed",
            async move {
                let room = chat.room(&room_id).await?;
                match reply_to {
                    Some(event_id) => {
                        room.send_image_reply(upload, &event_id).await?;
                        // 图片不带输入框里的字，草稿的正文留着，回复对象用掉了。
                        chat.enqueue_draft(DraftWrite::ForgetReply(room_id, event_id));
                    }
                    None => room.send_image(upload).await?,
                }
                Ok(())
            },
            move || onboarding::record_sent(&sent, sent_room),
        );
        Ok(())
    }
}

/// 等登录出结果：登录好了返回真；停在要用户处理的失败上、或者一直没动静过了期限，返回假。
/// （传完文件发链接时用：App 重启后 Matrix 可能还没登录好，等它，别让主人点重发。）
pub(super) async fn signed_in_within(
    mut view: tokio::sync::watch::Receiver<super::ChatViewModel>,
    wait: std::time::Duration,
) -> bool {
    let deadline = std::time::Instant::now() + wait;
    loop {
        match view.borrow().account {
            ChatAccount::SignedIn { .. } => return true,
            // 没在登录、也不会自己登录的，等也没用。
            ChatAccount::SignedOut | ChatAccount::Failed { .. } => return false,
            _ => {}
        }
        // 等变化也卡在期限内：一直没动静（登录卡住了）就到期走人。
        let Some(remaining) = deadline.checked_duration_since(std::time::Instant::now()) else {
            return false;
        };
        if tokio::time::timeout(remaining, view.changed())
            .await
            .is_err()
        {
            return false;
        }
    }
}

/// 附件区（ADR-0094）用到的聊天这一侧。
impl ChatLink {
    /// 点发送时的检查，和发文字一样：房间能发、回复对象的格式对。
    pub(crate) fn check_send(
        &self,
        room_id: &str,
        reply_to: Option<&str>,
    ) -> Result<(), CoreError> {
        reply_target(reply_to)?;
        self.send_target(room_id).map(drop)
    }

    /// 点了发送：字已经交给附件区，草稿（连同回复对象）留着没用了；发送途中又打的字不算。
    pub(crate) fn forget_sent_draft(&self, room_id: &str, body: &str) {
        if let Ok((_, room_id)) = self.target(room_id) {
            self.enqueue_draft(DraftWrite::Forget(room_id, body.trim().to_owned()));
        }
    }

    /// 这一组交给了发送队列：引导提示记下这个房间发过消息。
    pub(crate) fn record_sent_room(&self, room_id: &str) {
        onboarding::record_sent(&self.sent, room_id.to_owned());
    }
}
