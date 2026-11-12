//! Agent 进度状态消息（ADR-0082）的两个发送出口：首发走原始发送，马上拿事件 ID 去
//! 登记；编辑走发送队列，和轮次的答案同一 FIFO，`done` 那次编辑不会抢先于答案落地
//! （实测过原始发送会把编辑超到答案前面）。

use matrix_sdk::ruma::{OwnedEventId, events::room::message::RoomMessageEventContent};
use serde_json::Value;

use super::ChatRoom;
use crate::Result;

impl ChatRoom {
    /// 发一条内容任意的 `m.room.message`（不经发送队列），返回服务器的事件 ID。
    /// 内容原样发出，由调用方组装（状态消息的自定义字段与关系）；加密房间自动加密。
    pub async fn send_message_event(&self, content: Value) -> Result<OwnedEventId> {
        let sent = self
            .room
            .send_raw("m.room.message", content)
            .await
            .map_err(|error| crate::Error::Upstream(format!("发送失败：{error}")))?;
        Ok(sent.response.event_id)
    }

    /// 编辑一条已发出的消息（内容里已带 `m.new_content` 和 `m.relates_to`）：走发送
    /// 队列，和普通消息同一 FIFO；`msgtype`/`body` 进类型化内容，其余键原样并入顶层。
    pub async fn queue_message_edit(&self, content: Value) -> Result<()> {
        let (inner, extra) = split_content(content)?;
        self.timeline()
            .await
            .send_with_extra_content(inner.into(), Some(extra))
            .await?;
        Ok(())
    }
}

/// 把编辑内容拆成（类型化正文，其余顶层键）。`msgtype` 认 `m.text`，其余按 `m.notice`。
fn split_content(
    content: Value,
) -> Result<(RoomMessageEventContent, serde_json::Map<String, Value>)> {
    let object = content
        .as_object()
        .ok_or_else(|| crate::Error::InvalidInput("状态消息内容不是对象".into()))?;
    let body = object
        .get("body")
        .and_then(Value::as_str)
        .ok_or_else(|| crate::Error::InvalidInput("状态消息缺 body".into()))?;
    let inner = match object.get("msgtype").and_then(Value::as_str) {
        Some("m.text") => RoomMessageEventContent::text_plain(body),
        _ => RoomMessageEventContent::notice_plain(body),
    };
    let extra: serde_json::Map<String, Value> = object
        .iter()
        .filter(|(name, _)| name.as_str() != "msgtype" && name.as_str() != "body")
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    Ok((inner, extra))
}
