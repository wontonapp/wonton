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

use matrix_sdk_ui::timeline::{EventSendState, MsgLikeKind};

use super::{ChatRoom, ensure_recipient_devices};
use crate::{Error, Result};

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

impl ChatRoom {
    /// 重发所有被服务器拒收、卡在队首的消息（不可恢复的发送失败），按原来的顺序，再打开发送队列。返回重发了几条。
    ///
    /// 只给拒收原因已经消失的调用方用（比如网关的跳数被人说话清零了）：原因还在时会再被拒收一次，照样卡住。
    pub async fn resend_refused(&self) -> Result<usize> {
        let mut resent = 0;
        for item in self.timeline().await.items().await.iter() {
            let Some(event) = item.as_event() else {
                continue;
            };
            let refused = matches!(
                event.send_state(),
                Some(EventSendState::SendingFailed {
                    is_recoverable: false,
                    ..
                })
            );
            let Some(handle) = event.local_echo_send_handle().filter(|_| refused) else {
                continue;
            };
            handle
                .unwedge()
                .await
                .map_err(|error| Error::Upstream(format!("重发失败：{error}")))?;
            resent += 1;
        }
        // `unwedge` 只唤醒队列，出错后停下的队列要重新打开（见 `send_queue`）。
        if resent > 0 {
            self.room.send_queue().set_enabled(true);
        }
        Ok(resent)
    }

    /// 开旁聊的第一步（ADR-0093 §4）：发根消息（带 `app.wonton.side_chat` 的 `m.notice`，正文就是标题），
    /// 不经发送队列、等服务器回事件 ID。发不出去时按原因返回错误，房间里什么也不留下。
    pub async fn send_side_chat_root(&self, title: &str) -> Result<OwnedEventId> {
        ensure_recipient_devices(&self.room).await?;
        let mut content = serde_json::json!({ "msgtype": "m.notice", "body": title });
        content[crate::SIDE_CHAT_FIELD] = serde_json::json!({ "v": 1, "title": title });
        // 主人在等这一步：重试几次就报错，不像发送队列那样一直重试（5xx 时 SDK 默认会重试很久）。
        let sent = self
            .room
            .send_raw("m.room.message", content)
            .with_request_config(matrix_sdk::config::RequestConfig::short_retry())
            .await?;
        Ok(sent.response.event_id)
    }

    /// 把一条文字放进话题 `root` 的发送队列：本地回显照常出现在这个话题的时间线里，发送失败的照常重发或取消。
    /// 话题里还没有别的消息，回退的 `m.in_reply_to` 指向根。
    pub async fn queue_text_in_thread(&self, root: &EventId, body: &str) -> Result<()> {
        let thread = Thread::plain(root.to_owned(), root.to_owned());
        let content = RoomMessageEventContent::text_plain(body).make_for_thread(
            // `AddMentions::No` 下不读 sender，填自己只为满足签名。
            ReplyMetadata::new(root, self.room.own_user_id(), Some(&thread)),
            ReplyWithinThread::No,
            AddMentions::No,
        );
        self.room
            .send_queue()
            .send(content.into())
            .await
            .map_err(|error| crate::Error::Upstream(format!("放不进发送队列：{error}")))?;
        Ok(())
    }
}
