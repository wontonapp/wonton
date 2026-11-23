//! 长按消息的菜单要做的事（ADR-0073、ADR-0109 第 10 节）：回复、转发、撤回、编辑，以及取回复引用的原消息。

use matrix_sdk::{
    room::{
        edit::EditedContent,
        reply::{EnforceThread, Reply},
    },
    ruma::{
        EventId, RoomId,
        events::room::message::{
            AddMentions, MessageType, RoomMessageEventContent,
            RoomMessageEventContentWithoutRelation,
        },
    },
};
use matrix_sdk_ui::timeline::TimelineEventItemId;

use matrix_sdk::deserialized_responses::TimelineEvent;
use matrix_sdk::ruma::events::Mentions;

use super::{ChatRoom, ensure_recipient_devices};
use crate::{Error, MatrixSession, Result, SendFailure, SendOutcome, reply::load_event};

/// 事件是谁发的；解析不了的消息类事件返回 `None`。
fn sender(event: &TimelineEvent) -> Option<matrix_sdk::ruma::OwnedUserId> {
    match event.raw().deserialize().ok()? {
        matrix_sdk::ruma::events::AnySyncTimelineEvent::MessageLike(event) => {
            Some(event.sender().to_owned())
        }
        _ => None,
    }
}

fn mentions_of(users: &[matrix_sdk::ruma::OwnedUserId]) -> Mentions {
    Mentions::with_user_ids(users.iter().cloned())
}

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
    /// [`Error::ReplyTarget`]。
    pub async fn send_reply(
        &self,
        body: &str,
        mentions: &[String],
        in_reply_to: &EventId,
    ) -> Result<()> {
        self.send_mentions_reply(body, mentions, in_reply_to, None)
            .await
    }

    /// 回复和 [`Self::mention_content`](super::ChatRoom::mention_content) 一样带上选中的人和胶囊；被回复的人
    /// 也算叫到，一起写进 `m.mentions`。
    async fn send_mentions_reply(
        &self,
        body: &str,
        mentions: &[String],
        in_reply_to: &EventId,
        extra: Option<serde_json::Map<String, serde_json::Value>>,
    ) -> Result<()> {
        let quoted = self.check_reply_target(in_reply_to).await?;
        ensure_recipient_devices(&self.room).await?;
        let names = self.names();
        let mut users = crate::mentions::selected(mentions, &names);
        if let Some(sender) = sender(&quoted)
            && !names.is_own(&sender)
            && names.contains(&sender)
            && !users.contains(&sender)
        {
            users.push(sender);
        }
        let content = self.mention_content(body, &users);
        let reply = Reply {
            event_id: in_reply_to.to_owned(),
            enforce_thread: EnforceThread::MaybeThreaded,
            // 被回复的人这里自己加减过，不让 SDK 再按配置决定。
            add_mentions: AddMentions::No,
        };
        let mut content = self
            .room
            .make_reply_event(
                RoomMessageEventContentWithoutRelation::new(content.msgtype),
                reply,
            )
            .await
            .map_err(|error| Error::Upstream(format!("组回复失败：{error}")))?;
        if !users.is_empty() {
            content = content.add_mentions(mentions_of(&users));
        }
        self.timeline()
            .await
            .send_with_extra_content(content.into(), extra)
            .await?;
        Ok(())
    }

    /// 回复一条消息，事件内容里带上额外字段（链接预览的 `com.beeper.linkpreviews`，ADR-0065）。其余同
    /// [`Self::send_reply`]。
    pub async fn send_reply_with_extra(
        &self,
        body: &str,
        mentions: &[String],
        in_reply_to: &EventId,
        extra: serde_json::Map<String, serde_json::Value>,
    ) -> Result<()> {
        self.send_mentions_reply(body, mentions, in_reply_to, Some(extra))
            .await
    }

    /// 被回复的事件在这个房间里，否则返回 [`Error::ReplyTarget`]。SDK 组回复时取不到原事件只报一个笼统的错，
    /// 分不出是输入不对还是连不上，所以先自己取一次（本机有就不走网络）。
    pub(super) async fn check_reply_target(&self, in_reply_to: &EventId) -> Result<TimelineEvent> {
        load_event(&self.room, in_reply_to).await
    }

    /// 撤回一条消息（Matrix 的 redaction），不带理由。直接发给服务器，不走发送队列：结果当场知道，没权限、连不上
    /// 都原样返回；走队列的话失败只出现在队列里，时间线还会先把它显示成已撤回。
    pub async fn redact_message(&self, event_id: &EventId) -> Result<()> {
        self.room.redact(event_id, None, None).await?;
        Ok(())
    }

    /// 编辑一条已经发出去的文字（`m.replace`，ADR-0109 第 10 节）：所有人那里整条换成新正文，时间不变。
    /// 新正文按原样记成 `m.text`。`event_id` 是原来那条的事件 ID。
    pub async fn edit_message(&self, event_id: &str, body: &str) -> Result<()> {
        let event_id = event_id
            .parse()
            .map_err(|_| Error::InvalidInput(format!("不是事件 ID：{event_id}")))?;
        ensure_recipient_devices(&self.room).await?;
        let content = RoomMessageEventContentWithoutRelation::new(
            RoomMessageEventContent::text_plain(body).msgtype,
        );
        self.timeline()
            .await
            .edit(
                &TimelineEventItemId::EventId(event_id),
                EditedContent::RoomMessage(content),
            )
            .await?;
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
    /// 也在那里。附言先去首尾空白，只含空白的等于没带；有 `comment` 时先在 `to` 里发附言，服务器收下才转发，附言不可
    /// 恢复地失败（连发送队列都进不去也算）就不转发，原因进错误。为此读 `from` 的时间线、打开 `to` 的时间线并给它挂上补全
    /// 成员资料的后台任务，房间的 Sliding Sync 订阅不动；时间线建不起来时在发任何东西之前就失败。
    pub async fn forward(
        &self,
        from: &ChatRoom,
        event_id: &EventId,
        to: &RoomId,
        comment: Option<&str>,
    ) -> Result<()> {
        let content = from.forward_content(event_id).await?;
        let room = ChatRoom::open(self.joined(to)?).await?;
        match comment.map(str::trim).filter(|comment| !comment.is_empty()) {
            Some(comment) => {
                let not_queued =
                    |error: Error| Error::CommentNotSent(SendFailure::NotQueued(error.to_string()));
                let (_, updates) = room.send_updates().await.map_err(not_queued)?;
                let handle = room.send_text(comment, &[]).await.map_err(not_queued)?;
                match room.wait_outcome(updates, handle).await {
                    SendOutcome::Sent => {}
                    SendOutcome::Failed(failure) => return Err(Error::CommentNotSent(failure)),
                }
            }
            // 附言走 `send_text`，它会自己确保收件设备；不带附言直接转发这条要在这里补。
            None => ensure_recipient_devices(&room.room).await?,
        }
        room.room
            .send_queue()
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
