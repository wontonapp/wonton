//! 往房间发消息、组回复、等发送队列。

use matrix_sdk::ruma::{RoomId, events::room::message::RoomMessageEventContent};

use super::join::ROOM_SYNC_WAIT;
use crate::{Error, MatrixSession, Result};

impl MatrixSession {
    pub async fn send_text_to(&self, room_id: &RoomId, body: &str) -> Result<()> {
        let room = self.joined_within(room_id, ROOM_SYNC_WAIT).await?;
        room.send(RoomMessageEventContent::text_plain(body)).await?;
        Ok(())
    }
    pub async fn prepare_reply(
        &self,
        room_id: &RoomId,
        body: &str,
        event_id: Option<&matrix_sdk::ruma::EventId>,
        thread_root: Option<&matrix_sdk::ruma::EventId>,
    ) -> Result<crate::PreparedReply> {
        use matrix_sdk::room::reply::{EnforceThread, Reply};
        use matrix_sdk::ruma::events::room::message::{
            AddMentions, Relation, ReplyWithinThread, RoomMessageEventContentWithoutRelation,
        };
        let room = self.joined_within(room_id, ROOM_SYNC_WAIT).await?;
        let Some(event_id) = event_id.or(thread_root) else {
            return Ok(crate::PreparedReply(Box::new(
                RoomMessageEventContent::text_plain(body),
            )));
        };
        crate::reply::load_event(&room, event_id).await?;
        let enforce_thread = match thread_root {
            Some(_) => EnforceThread::Threaded(ReplyWithinThread::Yes),
            None => EnforceThread::Unthreaded,
        };
        let content = room
            .make_reply_event(
                RoomMessageEventContentWithoutRelation::text_plain(body),
                Reply {
                    event_id: event_id.to_owned(),
                    enforce_thread,
                    add_mentions: AddMentions::Yes,
                },
            )
            .await
            .map_err(|error| Error::Upstream(format!("组回复失败：{error}")))?;
        if let Some(root) = thread_root
            && !matches!(&content.relates_to, Some(Relation::Thread(thread)) if thread.event_id.as_str() == root.as_str())
        {
            return Err(Error::InvalidInput(
                "Reply target belongs to a different thread".into(),
            ));
        }
        Ok(crate::PreparedReply(Box::new(content)))
    }
    pub async fn wait_for_queued_sends(&self, room_id: &RoomId) -> Result<()> {
        use matrix_sdk::send_queue::{LocalEchoContent, RoomSendQueueUpdate};
        let room = self.joined(room_id)?;
        let queue = room.send_queue();
        let mut updates = room.client().send_queue().subscribe();
        loop {
            let (pending, _) = queue
                .subscribe()
                .await
                .map_err(|error| Error::Upstream(format!("读不了待发消息：{error}")))?;
            if pending.is_empty()
                || pending.iter().any(|echo| {
                    matches!(
                        &echo.content,
                        LocalEchoContent::Event {
                            send_error: Some(_),
                            ..
                        }
                    )
                })
            {
                return Ok(());
            }
            match updates.recv().await {
                Ok(update)
                    if update.room_id == room_id
                        && matches!(update.update, RoomSendQueueUpdate::SendError { .. }) =>
                {
                    return Ok(());
                }
                Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return Ok(()),
            }
        }
    }
}
