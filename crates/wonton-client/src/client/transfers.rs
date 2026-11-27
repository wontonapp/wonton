//! 文件与传输：传输列表、消息下的文件卡片和给播放器的地址，拆出 [client mod] 保持文件在门禁内。

use super::*;

#[uniffi::export]
impl ClientCore {
    /// 给 [`UploadSource::Opaque`](wonton_protocol::UploadSource::Opaque) 的来源换文件描述符。壳在创建核心之后、发出
    /// 第一个操作之前设置：App 重启后核心要接着传。
    pub fn set_file_opener(&self, opener: Arc<dyn FileOpener>) {
        self.scope().files.set_opener(opener.clone());
        self.replay().opener = Some(opener);
    }
    /// 当前的传输列表，用于首帧渲染。
    pub fn transfers(&self) -> TransfersViewModel {
        self.views().transfers.borrow().clone()
    }
    /// 这个房间的传输条列什么。不走网络，传输列表一变就可以重新取。
    /// 附件区自己的上传不在里面，它们画在附件区里。
    pub fn room_transfers(&self, room_id: String) -> RoomTransfers {
        let scope = self.scope();
        let listed = scope
            .files
            .view()
            .for_room(&room_id, &scope.chat.view().rooms);
        scope.composer.hide_own_uploads(listed)
    }
    /// 订阅传输列表，语义同 [`subscribe_session_list`](Self::subscribe_session_list)。
    pub fn subscribe_transfers(&self) -> Arc<TransfersSubscription> {
        Arc::new(TransfersSubscription::new(
            self.views().transfers.subscribe(),
        ))
    }
    /// 消息下面的文件卡片：取信息并缓存。结果写在卡片里，壳等待时画 [`FileCard::Loading`]。
    pub async fn file_card(&self, id: FileId) -> FileCard {
        let scope = self.scope();
        let files = scope.files.clone();
        let card = scope.run(async move { Ok(files.card(id).await) }).await;
        card.unwrap_or_else(|error| FileCard::Failed {
            reason: FileFailure::Other,
            message: error.to_string(),
        })
    }
    /// 卡片上的按钮和下载进展。不走网络，传输列表一变就可以重新取。
    pub fn file_card_actions(&self, info: FileInfo) -> FileCardActions {
        self.scope().files.card_actions(&info)
    }
    /// 文件的缩略图：JPEG 字节。不在访问范围里时返回 `Unauthorized`；文件没了、这个类型没有缩略图时返回 `InvalidInput`。
    pub async fn file_thumbnail(&self, id: FileId) -> Result<Vec<u8>, CoreError> {
        let scope = self.scope();
        let files = scope.files.clone();
        scope.run(async move { files.thumbnail(id).await }).await
    }
    /// 给 App 自己的播放器用的回环地址，带 `Range`：播放器不能给单个请求设代理。令牌每次启动换一个；App 回到前台后要重新取，
    /// 端口可能变了。
    pub async fn file_stream_url(&self, id: FileId) -> Result<String, CoreError> {
        let scope = self.scope();
        let files = scope.files.clone();
        scope.run(async move { files.stream_url(id).await }).await
    }
    /// 播放器放不了时问为什么：连不上家、家里的服务器报错，还是这台手机不支持这个格式。只有格式放不了时，
    /// 说明里才提可以下载后用其他 App 打开。
    pub async fn file_playback_problem(&self, id: FileId) -> FileProblem {
        let scope = self.scope();
        let files = scope.files.clone();
        let asked = id.clone();
        let problem = scope
            .run(async move { Ok(files.playback_problem(asked).await) })
            .await;
        problem.unwrap_or_else(|error| FileProblem {
            operation: FileOperation::Play,
            file: id,
            reason: FileFailure::Other,
            message: error.to_string(),
        })
    }
}
