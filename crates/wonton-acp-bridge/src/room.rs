//! 桥接层要对房间做的事。生产实现是 [`ChatRoom`]，测试用内存替身，好让桥的行为测试
//! 断言 Matrix 侧的副作用而不连服务器。

use std::future::Future;

use futures_util::Stream;
use serde_json::Value;
use wonton_matrix::{ChatRoom, Message, OwnedEventId};

use crate::error::{Detail, Error, Result};

pub trait Room: Send + Sync + 'static {
    fn room_id(&self) -> String;
    /// 发到主对话。
    fn send_text(&self, text: &str) -> impl Future<Output = Result<()>> + Send;
    /// 发到主对话并返回事件 ID（任务话题的根）。
    fn send_text_event(&self, text: &str) -> impl Future<Output = Result<String>> + Send;
    /// 发进话题、提及主人（推送）。
    fn send_thread_mention(
        &self,
        root: &str,
        text: &str,
        owner: &wonton_matrix::OwnedUserId,
    ) -> impl Future<Output = Result<()>> + Send;
    /// 发进话题的 `m.notice`（不推送）。
    fn send_thread_notice(&self, root: &str, text: &str)
    -> impl Future<Output = Result<()>> + Send;
    /// 发进`root`开的话题。
    fn send_thread_text(&self, root: &str, text: &str) -> impl Future<Output = Result<()>> + Send;
    /// 发一条不推送的 `m.notice`。
    fn send_notice(&self, text: &str) -> impl Future<Output = Result<()>> + Send;
    /// 发一条构造好的 Agent 进度状态消息（ADR-0082：内容自带话题关系），返回事件 ID。
    fn send_run_status(&self, content: Value) -> impl Future<Output = Result<String>> + Send;
    /// 用 `m.replace` 编辑一条已发出的状态消息（内容里已带 `m.relates_to`）。
    fn replace_run_status(&self, content: Value) -> impl Future<Output = Result<()>> + Send;
    fn typing(&self, typing: bool) -> impl Future<Output = Result<()>> + Send;
    /// 对这条消息发已读回执：Agent 真的收到它了（ADR-0035）。
    fn read_receipt(&self, event_id: &str) -> impl Future<Output = Result<()>> + Send;
    fn redact(&self, event_id: &str, reason: &str) -> impl Future<Output = Result<()>> + Send;
    /// 凭媒体句柄下载附件（加密房间在本机解密）。句柄里的密钥不进日志。
    fn download_media(&self, token: &str) -> impl Future<Output = Result<Vec<u8>>> + Send;
    fn set_state(
        &self,
        event_type: &str,
        state_key: &str,
        content: Value,
    ) -> impl Future<Output = Result<()>> + Send;
    /// `m.room.create` 事件，取不到为 `None`。
    async fn created(&self) -> Result<Option<Value>> {
        Ok(None)
    }
    /// 消息列表流；关闭时循环结束。
    fn updates(&self) -> impl Future<Output = impl Stream<Item = Vec<Message>> + Unpin + Send>;
}

impl<R: Room> Room for std::sync::Arc<R> {
    fn room_id(&self) -> String {
        (**self).room_id()
    }

    async fn send_text(&self, text: &str) -> Result<()> {
        (**self).send_text(text).await
    }

    async fn send_text_event(&self, text: &str) -> Result<String> {
        (**self).send_text_event(text).await
    }

    async fn send_thread_mention(
        &self,
        root: &str,
        text: &str,
        owner: &wonton_matrix::OwnedUserId,
    ) -> Result<()> {
        (**self).send_thread_mention(root, text, owner).await
    }

    async fn send_thread_notice(&self, root: &str, text: &str) -> Result<()> {
        (**self).send_thread_notice(root, text).await
    }

    async fn send_thread_text(&self, root: &str, text: &str) -> Result<()> {
        (**self).send_thread_text(root, text).await
    }

    async fn send_notice(&self, text: &str) -> Result<()> {
        (**self).send_notice(text).await
    }

    async fn send_run_status(&self, content: Value) -> Result<String> {
        (**self).send_run_status(content).await
    }

    async fn replace_run_status(&self, content: Value) -> Result<()> {
        (**self).replace_run_status(content).await
    }

    async fn typing(&self, typing: bool) -> Result<()> {
        (**self).typing(typing).await
    }

    async fn read_receipt(&self, event_id: &str) -> Result<()> {
        (**self).read_receipt(event_id).await
    }

    async fn redact(&self, event_id: &str, reason: &str) -> Result<()> {
        (**self).redact(event_id, reason).await
    }

    async fn download_media(&self, token: &str) -> Result<Vec<u8>> {
        (**self).download_media(token).await
    }

    async fn set_state(&self, event_type: &str, state_key: &str, content: Value) -> Result<()> {
        (**self).set_state(event_type, state_key, content).await
    }

    async fn created(&self) -> Result<Option<Value>> {
        (**self).created().await
    }

    fn updates(&self) -> impl Future<Output = impl Stream<Item = Vec<Message>> + Unpin + Send> {
        (**self).updates()
    }
}

impl Room for ChatRoom {
    fn room_id(&self) -> String {
        self.room_id().to_string()
    }

    async fn send_text(&self, text: &str) -> Result<()> {
        Ok(ChatRoom::send_text(self, text).await?)
    }

    async fn send_text_event(&self, text: &str) -> Result<String> {
        Ok(ChatRoom::send_text_event(self, text).await?.to_string())
    }

    async fn send_thread_mention(
        &self,
        root: &str,
        text: &str,
        owner: &wonton_matrix::OwnedUserId,
    ) -> Result<()> {
        let root: OwnedEventId = root
            .parse()
            .map_err(|_| Error::InvalidInput(Detail::data(format!("不是事件 ID：{root}"))))?;
        Ok(ChatRoom::send_thread_mention(self, &root, text, owner.as_ref()).await?)
    }

    async fn send_thread_notice(&self, root: &str, text: &str) -> Result<()> {
        let root: OwnedEventId = root
            .parse()
            .map_err(|_| Error::InvalidInput(Detail::data(format!("不是事件 ID：{root}"))))?;
        Ok(ChatRoom::send_thread_notice(self, &root, text).await?)
    }

    async fn send_thread_text(&self, root: &str, text: &str) -> Result<()> {
        let root: wonton_matrix::OwnedEventId = root
            .parse()
            .map_err(|_| Error::InvalidInput(Detail::data(format!("不是事件 ID：{root}"))))?;
        Ok(ChatRoom::send_text_in_thread(self, &root, text).await?)
    }

    async fn send_notice(&self, text: &str) -> Result<()> {
        Ok(ChatRoom::send_notice(self, text).await?)
    }

    async fn send_run_status(&self, content: Value) -> Result<String> {
        Ok(ChatRoom::send_message_event(self, content)
            .await?
            .to_string())
    }

    async fn replace_run_status(&self, content: Value) -> Result<()> {
        ChatRoom::queue_message_edit(self, content).await?;
        Ok(())
    }

    async fn typing(&self, typing: bool) -> Result<()> {
        Ok(ChatRoom::typing_notice(self, typing).await?)
    }

    async fn read_receipt(&self, event_id: &str) -> Result<()> {
        ChatRoom::send_read_receipt(self, event_id).await?;
        Ok(())
    }

    async fn redact(&self, event_id: &str, reason: &str) -> Result<()> {
        Ok(ChatRoom::redact(self, event_id, reason).await?)
    }

    async fn download_media(&self, token: &str) -> Result<Vec<u8>> {
        Ok(ChatRoom::download_media(self, token).await?)
    }

    async fn set_state(&self, event_type: &str, state_key: &str, content: Value) -> Result<()> {
        Ok(self
            .send_state_event_raw(event_type, state_key, content)
            .await?)
    }

    async fn created(&self) -> Result<Option<Value>> {
        Ok(self.fetch_state_event("m.room.create", "").await?)
    }

    async fn updates(&self) -> impl Stream<Item = Vec<Message>> + Unpin + Send {
        ChatRoom::message_updates(self).await
    }
}
