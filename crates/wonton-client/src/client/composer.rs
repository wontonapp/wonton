//! 输入框的附件区和一起发送（ADR-0094）。不返回结果的操作是 [`TimelineAction`] 的变体，经 `dispatch` 进来；要返回附件标识、
//! 预览图或订阅的是 `ClientCore` 的方法。都按 [`TimelineTarget`] 给出时间线，旁聊（`thread_root` 不为空）还不能用。

use std::sync::Arc;

use wonton_l10n::l10n;
use wonton_protocol::{CoreError, ImageUpload, TimelineAction, TimelineTarget, UploadSource};

use super::ClientCore;
use crate::l10n::{CATALOG, language};
use crate::scope::Scope;
use crate::subscription::ComposerSubscription;

/// 附件区的操作：整组交给 [`ComposerLink`](crate::composer::ComposerLink)，别的时间线操作照旧交给聊天。
pub(super) fn is_composer_action(action: &TimelineAction) -> bool {
    matches!(
        action,
        TimelineAction::SendComposer { .. }
            | TimelineAction::RemoveAttachment { .. }
            | TimelineAction::RetryAttachment { .. }
            | TimelineAction::RetryOutgoing { .. }
            | TimelineAction::CancelOutgoing { .. }
    )
}

/// 附件区按房间记，只管主对话：旁聊里的附件区另行实现，这之前旁聊里发文字和图片用
/// [`TimelineAction::SendText`]、[`TimelineAction::SendImage`]。
fn composer_room(target: &TimelineTarget) -> Result<String, CoreError> {
    if target.thread_root.is_some() {
        return Err(CoreError::NotReady {
            reason: l10n!(CATALOG, language(), "composer-side-chat-unsupported"),
        });
    }
    Ok(target.room_id.clone())
}

impl ClientCore {
    pub(super) fn composer_action(
        &self,
        scope: &Scope,
        action: TimelineAction,
    ) -> Result<(), CoreError> {
        let composer = &scope.composer;
        match action {
            TimelineAction::SendComposer {
                target,
                body,
                reply_to,
            } => composer.send(&composer_room(&target)?, &body, reply_to),
            TimelineAction::RemoveAttachment {
                target,
                attachment_id,
            } => composer.remove(&composer_room(&target)?, &attachment_id),
            TimelineAction::RetryAttachment {
                target,
                attachment_id,
            } => composer.retry(&composer_room(&target)?, &attachment_id),
            TimelineAction::RetryOutgoing {
                target,
                outgoing_id,
            } => composer.retry_outgoing(&composer_room(&target)?, &outgoing_id),
            TimelineAction::CancelOutgoing {
                target,
                outgoing_id,
            } => composer.cancel_outgoing(&composer_room(&target)?, &outgoing_id),
            // dispatch 只把 [`is_composer_action`] 认的交过来。
            _ => Err(CoreError::InvalidInput {
                reason: l10n!(CATALOG, language(), "chat-invalid-action"),
            }),
        }
    }
}

#[uniffi::export]
impl ClientCore {
    /// 订阅一条时间线的输入框：附件区和点了发送、还没交出去的消息。第一次 `next()` 立即返回当前值。附件区存在本机，没登录
    /// 也能看；换服务器后订阅结束（`next()` 返回空），壳按新服务器的房间重新订阅。旁聊返回 `NotReady`。
    pub fn subscribe_composer(
        &self,
        target: TimelineTarget,
    ) -> Result<Arc<ComposerSubscription>, CoreError> {
        let room_id = composer_room(&target)?;
        let receiver = self.scope().composer.subscribe(&room_id);
        Ok(Arc::new(ComposerSubscription::new(receiver)))
    }

    /// 把一张照片放进这条时间线的附件区，马上开始上传，返回附件的标识。不是图片类型、附件区满了时返回 `InvalidInput`；
    /// 还没登录、旁聊时返回 `NotReady`；存不下时返回 `Storage`。比服务器的上传上限大的照片放得进来，上传时标成失败
    /// （[`AttachmentFailure::TooLarge`](wonton_protocol::AttachmentFailure::TooLarge)）。上传的进展在
    /// [`ComposerViewModel`](wonton_protocol::ComposerViewModel) 里。
    pub fn add_image_attachment(
        &self,
        target: TimelineTarget,
        image: ImageUpload,
    ) -> Result<String, CoreError> {
        let room_id = composer_room(&target)?;
        self.scope().composer.add_image(&room_id, image)
    }

    /// 把一个文件放进附件区，马上经文件服务（tus）开始上传，返回附件的标识。`source`、`name`、`mime_type` 同
    /// [`FileAction::UploadFile`](wonton_protocol::FileAction::UploadFile)。名字为空、附件区满了时返回 `InvalidInput`。
    pub fn add_file_attachment(
        &self,
        target: TimelineTarget,
        source: UploadSource,
        name: String,
        mime_type: Option<String>,
    ) -> Result<String, CoreError> {
        let room_id = composer_room(&target)?;
        self.scope()
            .composer
            .add_file(&room_id, source, name, mime_type)
    }

    /// 附件区或待发消息里一张照片的预览图：壳给过缩略图就是它，否则是原图。文件和找不到的附件返回 `InvalidInput`。
    pub async fn attachment_thumbnail(
        &self,
        target: TimelineTarget,
        attachment_id: String,
    ) -> Result<Vec<u8>, CoreError> {
        let room_id = composer_room(&target)?;
        let scope = self.scope();
        let composer = scope.composer.clone();
        scope
            .run(async move {
                tokio::task::spawn_blocking(move || composer.thumbnail(&room_id, &attachment_id))
                    .await
                    .expect("reading a composer thumbnail does not panic")
            })
            .await
    }
}
