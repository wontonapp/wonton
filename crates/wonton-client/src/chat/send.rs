//! 发文字和图片，可以是对某条消息的回复（ADR-0073 第 2 节：回复的是哪条由壳每次显式给出）。

use std::sync::Arc;

use wonton_l10n::l10n;
use wonton_matrix::OwnedEventId;
use wonton_protocol::{ChatAccount, CoreError, ImageUpload};

use super::draft::DraftWrite;
use super::{ChatLink, TimelineKey, invalid, onboarding};
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
        key: &TimelineKey,
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
        self.send_target(key.room_id.as_str())?;
        let key = key.clone();
        let sent_room = key.room_id.to_string();
        self.spawn_op_with_success(
            "chat-op-send-text-failed",
            async move {
                let room = chat.room(&key).await?;
                // 预览的图片传不上去时不挡消息：正文照发，原因报出来（取不到的类别发送框里已经显示过）。
                let extra = match chat.preview_extra(&room, &key.room_id, &body).await {
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
                // 点掉的预览记录跟着作废：下次打同样的链接重新出预览。
                chat.previews.forget(&key.room_id);
                chat.enqueue_draft(DraftWrite::Forget(key, body));
                Ok(())
            },
            move || onboarding::record_sent(&sent, sent_room),
        );
        Ok(())
    }

    pub(super) fn send_image(
        self: &Arc<Self>,
        key: &TimelineKey,
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
        self.send_target(key.room_id.as_str())?;
        let key = key.clone();
        let sent_room = key.room_id.to_string();
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
                let room = chat.room(&key).await?;
                match reply_to {
                    Some(event_id) => {
                        room.send_image_reply(upload, &event_id).await?;
                        // 图片不带输入框里的字，草稿的正文留着，回复对象用掉了。
                        chat.enqueue_draft(DraftWrite::ForgetReply(key, event_id));
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
            self.enqueue_draft(DraftWrite::Forget(
                TimelineKey::main(room_id),
                body.trim().to_owned(),
            ));
        }
    }

    /// 这一组交给了发送队列：引导提示记下这个房间发过消息。
    pub(crate) fn record_sent_room(&self, room_id: &str) {
        onboarding::record_sent(&self.sent, room_id.to_owned());
    }
}
