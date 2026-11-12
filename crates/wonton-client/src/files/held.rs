//! 附件区里的文件（ADR-0094）：上传不带房间，传完不发链接。授权给房间和发消息由附件区在点了发送、这一组全部传完以后做。

use std::sync::Arc;

use wonton_protocol::{CoreError, FileFailure, FileId, TransferState, UploadSource};

use super::FilesLink;
use super::api::Api;

impl FilesLink {
    /// 开始上传，返回传输的标识。进展在传输列表里，断了照常接着传。
    pub(crate) fn start_held_upload(
        self: &Arc<Self>,
        source: UploadSource,
        name: String,
        mime_type: Option<String>,
    ) -> Result<String, CoreError> {
        let record = self.upload_record(source, name, mime_type, None)?;
        let id = record.id.clone();
        self.start(record)?;
        Ok(id)
    }

    /// 附件不要了：还在传的取消（连服务器上传了一半的一起删），做完或失败的从列表里清掉。
    pub(crate) fn drop_held_upload(self: &Arc<Self>, id: &str) {
        let Some(record) = self.store.get(id) else {
            return;
        };
        let result = match record.state {
            TransferState::Running | TransferState::Paused | TransferState::LinkNotSent { .. } => {
                self.cancel(id)
            }
            TransferState::Done | TransferState::Failed { .. } => self.dismiss(id, false),
        };
        if let Err(error) = result {
            tracing::info!(transfer = %id, %error, "could not drop the composer's upload");
        }
    }

    /// 把这个房间的 Bot 加进文件的访问范围。失败时给出类别和说明；连不上时类别是 `TransportUnavailable`。
    pub(crate) async fn grant_to_room(
        &self,
        file: &FileId,
        room: &str,
    ) -> Result<(), (FileFailure, String)> {
        let api = Api::connect(&self.home).map_err(|problem| problem.into_parts())?;
        api.grant(file, room)
            .await
            .map_err(|problem| problem.into_parts())
    }
}
