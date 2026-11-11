//! 长按消息的菜单要做的事（ADR-0073）：回复、转发、撤回，以及取回复引用的原消息。

use matrix_sdk::{
    room::reply::{EnforceThread, Reply},
    ruma::{
        EventId, RoomId,
        events::room::message::{
            AddMentions, MessageType, RoomMessageEventContent,
            RoomMessageEventContentWithoutRelation,
        },
    },
};

use super::{ChatRoom, ensure_recipient_devices};
use crate::{Error, MatrixSession, Result, reply::load_event};

impl ChatRoom {
    /// 时间线里这条回复引用的原消息不在本地时，向服务器取（SDK 的 `fetch_details_for_event`），结果随时间线的增量来。
    /// `reply_event_id` 是回复自己的事件 ID。这条还是本地回显（服务器的回显还没到）时 SDK 取不了，返回 `false`，
    /// 等它换成服务器的那条再来。
    pub async fn fetch_quoted(&self, reply_event_id: &EventId) -> Result<bool> {
        let timeline = self.timeline().await;
        let remote = timeline
            .item_by_event_id(reply_event_id)
            .await
            .is_some_and(|item| item.is_remote_event());
        if !remote {
            return Ok(false);
        }
        timeline.fetch_details_for_event(reply_event_id).await?;
        Ok(true)
    }

    /// 回复一条消息：发一条带 `m.in_reply_to` 的文字，其余同 [`Self::send_text`]。被回复的事件不在这个房间里时返回
    /// [`Error::InvalidInput`]。
    pub async fn send_reply(&self, body: &str, in_reply_to: &EventId) -> Result<()> {
        self.check_reply_target(in_reply_to).await?;
        ensure_recipient_devices(&self.room).await?;
        self.timeline()
            .await
            .send_reply(
                RoomMessageEventContentWithoutRelation::text_plain(body),
                in_reply_to.to_owned(),
            )
            .await?;
        Ok(())
    }

    /// 回复一条消息，事件内容里带上额外字段（链接预览的 `com.beeper.linkpreviews`，ADR-0065）。其余同
    /// [`Self::send_reply`]。
    pub async fn send_reply_with_extra(
        &self,
        body: &str,
        in_reply_to: &EventId,
        extra: serde_json::Map<String, serde_json::Value>,
    ) -> Result<()> {
        self.check_reply_target(in_reply_to).await?;
        ensure_recipient_devices(&self.room).await?;
        // 时间线的 `send_reply` 不收额外字段，这里自己组回复事件：不强制开话题、提及对方，和它推断出的一样。
        let reply = Reply {
            event_id: in_reply_to.to_owned(),
            enforce_thread: EnforceThread::MaybeThreaded,
            add_mentions: AddMentions::Yes,
        };
        let content = self
            .room
            .make_reply_event(
                RoomMessageEventContentWithoutRelation::text_plain(body),
                reply,
            )
            .await
            .map_err(|error| Error::Upstream(format!("组回复失败：{error}")))?;
        self.timeline()
            .await
            .send_with_extra_content(content.into(), Some(extra))
            .await?;
        Ok(())
    }

    /// 被回复的事件在这个房间里，否则返回 [`Error::InvalidInput`]。SDK 组回复时取不到原事件只报一个笼统的错，
    /// 分不出是输入不对还是连不上，所以先自己取一次（本机有就不走网络）。
    pub(super) async fn check_reply_target(&self, in_reply_to: &EventId) -> Result<()> {
        load_event(&self.room, in_reply_to).await.map(drop)
    }

    /// 撤回一条消息（Matrix 的 redaction），不带理由。直接发给服务器，不走发送队列：结果当场知道，没权限、连不上
    /// 都原样返回；走队列的话失败只出现在队列里，时间线还会先把它显示成已撤回。
    pub async fn redact_message(&self, event_id: &EventId) -> Result<()> {
        self.room.redact(event_id, None, None).await?;
        Ok(())
    }

    /// 转发要发的内容：时间线里这条的最新内容（编辑过的按编辑后），不带 `m.relates_to` 和提及。只有文字和图片能转发。
    ///
    /// 用的是 SDK 处理过的内容：旧式回复的回退正文已经去掉，图片的媒体地址和加密信息原样保留，不重新上传。
    async fn forward_content(&self, event_id: &EventId) -> Result<RoomMessageEventContent> {
        let item = self
            .timeline()
            .await
            .item_by_event_id(event_id)
            .await
            .ok_or_else(|| Error::InvalidInput(format!("时间线里没有这一条：{event_id}")))?;
        let message = item
            .content()
            .as_message()
            .ok_or_else(|| Error::InvalidInput("只能转发文字和图片".to_owned()))?;
        match message.msgtype() {
            msgtype @ (MessageType::Text(_) | MessageType::Notice(_) | MessageType::Image(_)) => {
                Ok(RoomMessageEventContent::new(msgtype.clone()))
            }
            _ => Err(Error::InvalidInput("只能转发文字和图片".to_owned())),
        }
    }
}

impl MatrixSession {
    /// 把 `from` 里的一条消息照原样发到已加入的房间 `to`：经那个房间的发送队列，本地回显出现在它的时间线里，发送结果
    /// 也在那里。不打开 `to`，不动 Sliding Sync 的房间订阅。
    pub async fn forward(&self, from: &ChatRoom, event_id: &EventId, to: &RoomId) -> Result<()> {
        let content = from.forward_content(event_id).await?;
        let room = self.joined(to)?;
        ensure_recipient_devices(&room).await?;
        room.send_queue()
            .send(content.into())
            .await
            .map_err(|error| Error::Upstream(format!("转发进不了发送队列：{error}")))?;
        Ok(())
    }

    /// 本账号已经加入了这个房间。
    pub fn is_joined(&self, room_id: &RoomId) -> bool {
        self.joined(room_id).is_ok()
    }
}
