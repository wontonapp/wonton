//! 输入框的附件区和一起发送（ADR-0094）。新操作是 `ClientCore` 的方法，不加 `Action`：`Action` 的 UniFFI 元数据已到 32 KB 上限。

use std::sync::Arc;

use wonton_protocol::{CoreError, ImageUpload, UploadSource};

use super::ClientCore;
use crate::subscription::ComposerSubscription;

#[uniffi::export]
impl ClientCore {
    /// 订阅一个房间的输入框：附件区和点了发送、还没交出去的消息。第一次 `next()` 立即返回当前值。附件区存在本机，没登录也能看；
    /// 换服务器后订阅结束（`next()` 返回空），壳按新服务器的房间重新订阅。
    pub fn subscribe_composer(
        &self,
        room_id: String,
    ) -> Result<Arc<ComposerSubscription>, CoreError> {
        let receiver = self.scope().composer.subscribe(&room_id);
        Ok(Arc::new(ComposerSubscription::new(receiver)))
    }

    /// 把一张照片放进这个房间的附件区，马上开始上传，返回附件的标识。不是图片类型、附件区满了时返回 `InvalidInput`；
    /// 还没登录时返回 `NotReady`；存不下时返回 `Storage`。比服务器的上传上限大的照片放得进来，上传时标成失败
    /// （[`AttachmentFailure::TooLarge`](wonton_protocol::AttachmentFailure::TooLarge)）。上传的进展在
    /// [`ComposerViewModel`](wonton_protocol::ComposerViewModel) 里。
    pub fn add_image_attachment(
        &self,
        room_id: String,
        image: ImageUpload,
    ) -> Result<String, CoreError> {
        self.scope().composer.add_image(&room_id, image)
    }

    /// 把一个文件放进附件区，马上经文件服务（tus）开始上传，返回附件的标识。`source`、`name`、`mime_type` 同
    /// [`Action::UploadFile`](wonton_protocol::Action::UploadFile)。名字为空、附件区满了时返回 `InvalidInput`。
    pub fn add_file_attachment(
        &self,
        room_id: String,
        source: UploadSource,
        name: String,
        mime_type: Option<String>,
    ) -> Result<String, CoreError> {
        self.scope()
            .composer
            .add_file(&room_id, source, name, mime_type)
    }

    /// 从附件区删掉一个附件，停下它的上传。已经传上去的照片留在服务器上，文件由家里的服务器到期清掉。附件区里没有它时返回
    /// `InvalidInput`。
    pub fn remove_attachment(
        &self,
        room_id: String,
        attachment_id: String,
    ) -> Result<(), CoreError> {
        self.scope().composer.remove(&room_id, &attachment_id)
    }

    /// 重新上传一个失败的附件；不是失败的不动。附件区和待发消息里都没有它时返回 `InvalidInput`。
    pub fn retry_attachment(
        &self,
        room_id: String,
        attachment_id: String,
    ) -> Result<(), CoreError> {
        self.scope().composer.retry(&room_id, &attachment_id)
    }

    /// 附件区或待发消息里一张照片的预览图：壳给过缩略图就是它，否则是原图。文件和找不到的附件返回 `InvalidInput`。
    pub async fn attachment_thumbnail(
        &self,
        room_id: String,
        attachment_id: String,
    ) -> Result<Vec<u8>, CoreError> {
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

    /// 把输入框的字和附件区的附件作为一次发送发出（ADR-0094）：马上变成一条待发消息，附件区和草稿清空；附件全部传完、文件授权给
    /// 房间后，才按顺序把这一组事件交给 Matrix。`reply_to` 同 [`Action::SendText`](wonton_protocol::Action::SendText)。
    /// 字和附件都没有、附件区里有失败的附件时返回 `InvalidInput`。
    pub fn send_composer(
        &self,
        room_id: String,
        body: String,
        reply_to: Option<String>,
    ) -> Result<(), CoreError> {
        self.scope().composer.send(&room_id, &body, reply_to)
    }

    /// 重发一条失败的待发消息：只重传失败的附件，再接着发。不是失败的不动；找不到时返回 `InvalidInput`。
    pub fn retry_outgoing(&self, room_id: String, outgoing_id: String) -> Result<(), CoreError> {
        self.scope().composer.retry_outgoing(&room_id, &outgoing_id)
    }

    /// 取消一条还没交出去的待发消息，整条不发，停下它的上传；排在它后面的接着发。找不到时（已经交出去了）返回
    /// `InvalidInput`，交出去的用 [`Action::CancelSend`](wonton_protocol::Action::CancelSend)。
    pub fn cancel_outgoing(&self, room_id: String, outgoing_id: String) -> Result<(), CoreError> {
        self.scope()
            .composer
            .cancel_outgoing(&room_id, &outgoing_id)
    }
}
