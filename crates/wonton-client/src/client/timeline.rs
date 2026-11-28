//! 房间与消息：解房间 ID、开时间线、草稿、旁聊和媒体，拆出 [client mod] 保持文件在门禁内。

use super::*;

#[uniffi::export]
impl ClientCore {
    /// 把 Matrix 房间 ID 或别名解成已加入的房间 ID；不自动加入房间。
    /// 取不到、未加入和输入非法分别返回对应的错误，壳不按别名等待一个永远不会出现的 ID。
    pub async fn resolve_chat_room(&self, input: String) -> Result<String, CoreError> {
        let scope = self.scope();
        let chat = scope.chat.clone();
        scope
            .run(async move { chat.resolve_chat_room(&input).await })
            .await
    }
    /// 订阅一条时间线（主对话或旁聊，ADR-0093 §5）：第一批是全部条目，之后是按顺序应用的增量。
    /// 可同时订阅多个房间或同一目标；滚动位置按订阅独立，取消一份不影响其他订阅。
    /// 还没登录、房间还没同步到本机时返回 `NotReady`。
    pub async fn subscribe_timeline(
        &self,
        target: TimelineTarget,
    ) -> Result<Arc<TimelineSubscription>, CoreError> {
        // 假数据入口（`new_ui_fake`）没有后端：时间线是写死的，只对界面测试约定的操作再推一批。
        #[cfg(feature = "ui-fake")]
        if self.ui_fake.load(Ordering::SeqCst) {
            return Ok(self.ui_fake_timelines.subscribe(&target));
        }
        let scope = self.scope();
        let chat = scope.chat.clone();
        let pages = scope.pages.clone();
        let opening = chat.begin_timeline(&target)?;
        scope
            .run(async move {
                let room_view = pages.subscribe_room_view(&target.room_id).await?;
                chat.open_timeline(opening, room_view).await
            })
            .await
    }
    /// 一条时间线（主对话或旁聊）输入框里的草稿（正文和正在回复的消息），没打过字也没在回复时是
    /// `None`。每个旁聊各有一份，换房间、重启核心都还在，主人回来接着写。
    pub async fn draft(&self, target: TimelineTarget) -> Result<Option<ComposerDraft>, CoreError> {
        // 假数据入口没有服务器，草稿由假时间线存（`ui_fake_timeline.rs`）。
        #[cfg(feature = "ui-fake")]
        if self.ui_fake.load(Ordering::SeqCst) {
            return Ok(self.ui_fake_timelines.draft(&target));
        }
        let scope = self.scope();
        let chat = scope.chat.clone();
        scope.run(async move { chat.draft(&target).await }).await
    }
    /// 开旁聊（ADR-0093 §4）：发根消息（带 `app.wonton.side_chat` 的 `m.notice`），等服务器回事件 ID，
    /// 把第一句话放进这个话题的发送队列，返回根的事件 ID。根没发出去时按原因返回错误（传输不可用、没就绪、
    /// 输入非法、服务器出错），房间里不留空旁聊。第一句话之后的发送结果在旁聊的时间线里，照常重发或取消。
    pub async fn start_side_chat(
        &self,
        room_id: String,
        body: String,
    ) -> Result<String, CoreError> {
        #[cfg(feature = "ui-fake")]
        if self.ui_fake.load(Ordering::SeqCst) {
            return Ok(self.ui_fake_timelines.start_side_chat(&body));
        }
        let scope = self.scope();
        let chat = scope.chat.clone();
        scope
            .run(async move { chat.start_side_chat(&room_id, &body).await })
            .await
    }
    /// 取图片等媒体的原文件（时间线里给出的句柄）。核心负责下载、解密和缓存。
    pub async fn load_media(&self, media: MediaHandle) -> Result<Vec<u8>, CoreError> {
        // 假数据入口没有后端：假房间图片的字节直接在这给，不进下载。
        #[cfg(feature = "ui-fake")]
        if self.ui_fake.load(Ordering::SeqCst)
            && let Some(bytes) = ui_fake_room::media(&media.token)
        {
            return Ok(bytes);
        }
        let scope = self.scope();
        let chat = scope.chat.clone();
        scope.run(async move { chat.load_media(media).await }).await
    }
    /// 链接预览卡片的图片原文件（凭卡片图片里的 id）。核心负责从家里取回来。
    pub async fn load_link_preview_image(&self, id: String) -> Result<Vec<u8>, CoreError> {
        let scope = self.scope();
        let chat = scope.chat.clone();
        scope
            .run(async move { chat.preview_image(&id).await })
            .await
    }
    /// 视频头像的本机缓存文件；相同媒体的并发请求共用一次下载。
    pub async fn avatar_file(&self, media: MediaHandle) -> Result<String, CoreError> {
        let scope = self.scope();
        let chat = scope.chat.clone();
        scope
            .run(async move { chat.avatar_file(media).await })
            .await
    }
}
