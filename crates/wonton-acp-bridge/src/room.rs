//! 桥接层要对房间做的事。生产实现是 [`ChatRoom`]，测试用内存替身，好让桥的行为测试
//! 断言 Matrix 侧的副作用而不连服务器。

use std::future::Future;

use futures_util::Stream;
use serde_json::Value;
use wonton_matrix::{ChatRoom, Message};

use crate::error::{Detail, Error, Result};

pub trait Room: Send + Sync + 'static {
    fn room_id(&self) -> String;
    /// 发到主对话。
    fn send_text(&self, text: &str) -> impl Future<Output = Result<()>> + Send;
    /// 发进`root`开的话题。
    fn send_thread_text(&self, root: &str, text: &str) -> impl Future<Output = Result<()>> + Send;
    /// 发一条不推送的 `m.notice`。
    fn send_notice(&self, text: &str) -> impl Future<Output = Result<()>> + Send;
    fn typing(&self, typing: bool) -> impl Future<Output = Result<()>> + Send;
    fn redact(&self, event_id: &str, reason: &str) -> impl Future<Output = Result<()>> + Send;
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

    async fn send_thread_text(&self, root: &str, text: &str) -> Result<()> {
        (**self).send_thread_text(root, text).await
    }

    async fn send_notice(&self, text: &str) -> Result<()> {
        (**self).send_notice(text).await
    }

    async fn typing(&self, typing: bool) -> Result<()> {
        (**self).typing(typing).await
    }

    async fn redact(&self, event_id: &str, reason: &str) -> Result<()> {
        (**self).redact(event_id, reason).await
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

    async fn send_thread_text(&self, root: &str, text: &str) -> Result<()> {
        let root: wonton_matrix::OwnedEventId = root
            .parse()
            .map_err(|_| Error::InvalidInput(Detail::data(format!("不是事件 ID：{root}"))))?;
        Ok(ChatRoom::send_text_in_thread(self, &root, text).await?)
    }

    async fn send_notice(&self, text: &str) -> Result<()> {
        Ok(ChatRoom::send_notice(self, text).await?)
    }

    async fn typing(&self, typing: bool) -> Result<()> {
        Ok(ChatRoom::typing_notice(self, typing).await?)
    }

    async fn redact(&self, event_id: &str, reason: &str) -> Result<()> {
        Ok(ChatRoom::redact(self, event_id, reason).await?)
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
