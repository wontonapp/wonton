//! 往话题里发消息和发 `m.notice`：和 [`ChatRoom::send_text`](super::ChatRoom::send_text) 一样走发送队列，
//! 本地回显先出现在时间线里，发送结果见 [`Message::status`](crate::Message::status)。

use matrix_sdk::ruma::{
    EventId, OwnedEventId,
    events::relation::Thread,
    events::room::message::{
        AddMentions, ReplyMetadata, ReplyWithinThread, RoomMessageEventContent,
    },
};

use super::ChatRoom;
use crate::Result;

impl ChatRoom {
    /// 发一条文字进指定话题，`root` 是话题根消息的事件 ID。
    ///
    /// 按 Matrix 规范的回退写法带 `m.in_reply_to`：指向本机时间线里话题最近一条已发出的消息，不知道时指向
    /// 根消息；`is_falling_back` 恒为 `true`（这条不是对话题里某条消息的真的回复）。
    pub async fn send_text_in_thread(&self, root: &EventId, body: &str) -> Result<()> {
        let latest = self.latest_sent_in_thread(root).await;
        let thread = Thread::plain(root.to_owned(), latest.clone());
        let content = RoomMessageEventContent::text_plain(body).make_for_thread(
            // `AddMentions::No` 下不读 sender，填自己只为满足签名。
            ReplyMetadata::new(&latest, self.room.own_user_id(), Some(&thread)),
            ReplyWithinThread::No,
            AddMentions::No,
        );
        self.timeline.send(content.into()).await?;
        Ok(())
    }

    /// 发一条 `m.notice`（状态说明）：Matrix 的推送规则默认不对 `m.notice` 响铃。
    pub async fn send_notice(&self, body: &str) -> Result<()> {
        self.timeline
            .send(RoomMessageEventContent::notice_plain(body).into())
            .await?;
        Ok(())
    }

    /// 本机时间线里话题最近一条已发出的消息的事件 ID；根消息自己也算话题里的一条。
    async fn latest_sent_in_thread(&self, root: &EventId) -> OwnedEventId {
        for item in self.timeline.items().await.into_iter().rev() {
            let Some(event) = item.as_event() else {
                continue;
            };
            // 没发出去的本地回显没有事件 ID，不算。
            let Some(id) = event.event_id() else { continue };
            let in_thread = id == root
                || event
                    .content()
                    .as_msglike()
                    .and_then(|c| c.thread_root.as_deref())
                    == Some(root);
            if in_thread {
                return id.to_owned();
            }
        }
        root.to_owned()
    }
}
