//! 往话题里发消息和发 `m.notice`：和 [`ChatRoom::send_text`](super::ChatRoom::send_text) 一样走发送队列，
//! 本地回显先出现在时间线里，发送结果见 [`Message::status`](crate::Message::status)。

use matrix_sdk::ruma::{
    EventId, OwnedEventId, UserId,
    events::Mentions,
    events::relation::Thread,
    events::room::message::{
        AddMentions, ReplyMetadata, ReplyWithinThread, RoomMessageEventContent,
    },
};

use matrix_sdk_ui::timeline::MsgLikeKind;

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
        self.timeline().await.send(content.into()).await?;
        Ok(())
    }

    /// 发一条 `m.notice`（状态说明）：Matrix 的推送规则默认不对 `m.notice` 响铃。
    pub async fn send_notice(&self, body: &str) -> Result<()> {
        self.timeline()
            .await
            .send(RoomMessageEventContent::notice_plain(body).into())
            .await?;
        Ok(())
    }

    /// 本机时间线里话题最近一条已发出的消息的事件 ID；根消息自己也算话题里的一条。
    async fn latest_sent_in_thread(&self, root: &EventId) -> OwnedEventId {
        for item in self.timeline().await.items().await.into_iter().rev() {
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

impl ChatRoom {
    /// 发一条普通文字并等它落到时间线，返回事件 ID（话题根要用它）。
    pub async fn send_text_event(&self, body: &str) -> Result<OwnedEventId> {
        self.timeline()
            .await
            .send(RoomMessageEventContent::text_plain(body).into())
            .await?;
        for _ in 0..100 {
            for item in self.timeline().await.items().await.into_iter().rev() {
                let Some(event) = item.as_event() else {
                    continue;
                };
                let Some(id) = event.event_id() else { continue };
                let matches = event
                    .content()
                    .as_msglike()
                    .and_then(|c| match &c.kind {
                        MsgLikeKind::Message(message) => Some(message.body()),
                        _ => None,
                    })
                    .is_some_and(|sent| sent == body);
                if matches && event.sender() == self.room.own_user_id() {
                    return Ok(id.to_owned());
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        Err(crate::Error::Upstream("发出去的消息没有落到时间线".into()))
    }

    /// 发一条会提及主人的文字进指定话题（推送会响）。
    pub async fn send_thread_mention(
        &self,
        root: &EventId,
        body: &str,
        owner: &UserId,
    ) -> Result<()> {
        let latest = self.latest_sent_in_thread(root).await;
        let thread = Thread::plain(root.to_owned(), latest.clone());
        let content = RoomMessageEventContent::text_plain(body)
            .make_for_thread(
                ReplyMetadata::new(&latest, self.room.own_user_id(), Some(&thread)),
                ReplyWithinThread::No,
                AddMentions::Yes,
            )
            .add_mentions(Mentions::with_user_ids([owner.to_owned()]));
        self.timeline().await.send(content.into()).await?;
        Ok(())
    }

    /// 发一条进指定话题的 `m.notice`（不推送）。
    pub async fn send_thread_notice(&self, root: &EventId, body: &str) -> Result<()> {
        let latest = self.latest_sent_in_thread(root).await;
        let thread = Thread::plain(root.to_owned(), latest.clone());
        let content = RoomMessageEventContent::notice_plain(body).make_for_thread(
            ReplyMetadata::new(&latest, self.room.own_user_id(), Some(&thread)),
            ReplyWithinThread::No,
            AddMentions::No,
        );
        self.timeline().await.send(content.into()).await?;
        Ok(())
    }
}
