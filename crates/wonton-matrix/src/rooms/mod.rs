//! 房间列表：按最近活动排序，带名字、最新消息预览和未读数；建私聊、加入和离开房间。
//!
//! 列表由 matrix-sdk-ui 的 `RoomListService` 维护（和 Element X 一样），排序也用它的：正在发送的在前，
//! 其余按最近活动，再按名字。

use crate::ChatRoom;
use crate::Draft;
use crate::Error;
use crate::MatrixSession;
use crate::MessageBody;
use crate::Result;
use crate::message::body;
use direct::direct_peer;
use futures_util::Stream;
use futures_util::StreamExt;
use futures_util::stream;
use join::ROOM_SYNC_WAIT;
use matrix_sdk::ComposerDraft;
use matrix_sdk::ComposerDraftType;
use matrix_sdk::Room;
use matrix_sdk::RoomDisplayName;
use matrix_sdk::RoomMemberships;
use matrix_sdk::RoomState;
use matrix_sdk::ruma::EventId;
use matrix_sdk::ruma::MilliSecondsSinceUnixEpoch;
use matrix_sdk::ruma::OwnedRoomId;
use matrix_sdk::ruma::OwnedUserId;
use matrix_sdk::ruma::RoomId;
use matrix_sdk::ruma::events::GlobalAccountDataEvent;
use matrix_sdk::ruma::events::direct::DirectEventContent;
use matrix_sdk::ruma::events::room::message::RoomMessageEventContent;
use matrix_sdk_ui::eyeball_im::Vector;
use matrix_sdk_ui::room_list_service::RoomListItem;
use matrix_sdk_ui::room_list_service::filters;
use matrix_sdk_ui::timeline::LatestEventValue;
use matrix_sdk_ui::timeline::Profile;
use matrix_sdk_ui::timeline::RoomExt;
use matrix_sdk_ui::timeline::TimelineDetails;
use matrix_sdk_ui::timeline::TimelineItemContent;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::pin::pin;
use std::sync::Arc;
use tokio::sync::watch;

mod direct;
mod join;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Membership {
    /// 已加入。
    Joined,
    /// 收到邀请，还没加入。
    Invited,
    /// 已离开。
    Left,
    /// 已申请加入，等待批准。
    Knocked,
    /// 被封禁。
    Banned,
}

impl From<RoomState> for Membership {
    fn from(state: RoomState) -> Self {
        match state {
            RoomState::Joined => Self::Joined,
            RoomState::Invited => Self::Invited,
            RoomState::Left => Self::Left,
            RoomState::Knocked => Self::Knocked,
            RoomState::Banned => Self::Banned,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoomInfo {
    /// 房间 ID。
    pub room_id: OwnedRoomId,
    /// 显示名：房间名，私聊时是对方的名字。
    pub name: String,
    /// 是否是私聊。
    pub is_direct: bool,
    /// 私聊对方：加入了的私聊是 `m.direct` 里唯一的对方，和 [`MatrixSession::direct_chat_with`] 认私聊的办法一样；
    /// 受邀还没加入的私聊是邀请人。
    pub direct_user_id: Option<OwnedUserId>,
    /// 是否开了端到端加密。
    pub is_encrypted: bool,
    /// 本账号的成员状态。
    pub membership: Membership,
    /// 建房时带了好友标记（ADR-0051）：邀请是一条好友请求，加入后是好友私聊。
    pub is_friend: bool,
    /// 最新一条消息；还没有消息或者还没算出来时为 `None`。
    pub latest: Option<LatestMessage>,
    /// 未读消息数，按本账号的已读回执在本机算出。
    pub unread_count: u64,
    /// 未读里提到本账号的条数（ADR-0121 的「@」徽标）；话题里的提及也算在房间头上。
    pub unread_mentions: u64,
    /// 房间里有人在通话，同 [`ChatRoom::call_active`](crate::ChatRoom::call_active)。
    pub call_active: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LatestMessage {
    /// 发送者的 Matrix 用户 ID。
    pub sender_id: OwnedUserId,
    /// 发送者的显示名。
    pub sender_name: String,
    /// 是否由本账号发出。
    pub is_own: bool,
    /// 内容；不是消息的事件（成员变化等）是 [`MessageBody::Other`]。
    pub body: MessageBody,
    /// 发送时间，Unix 毫秒。
    pub timestamp_ms: u64,
    /// 这一条属于旁聊（根、结果行、话题里的消息）时，旁聊的标题和结果行（ADR-0093 §4）。
    pub side_chat: Option<crate::side_chat::LatestSideChat>,
}

const PAGE_SIZE: usize = 500;

impl MatrixSession {
    pub async fn rooms(&self) -> Result<impl Stream<Item = Vec<RoomInfo>> + use<>> {
        let list = self
            .sync
            .room_list_service()
            .all_rooms()
            .await
            .map_err(|error| Error::Upstream(error.to_string()))?;
        // SDK 处理 `m.direct` 时不算房间的显著变化，列表不会因此重发这个房间；单独观察它，私聊标记变了也重算。
        let direct_changes = self
            .client
            .observe_events::<GlobalAccountDataEvent<DirectEventContent>, ()>();
        // 房间的未读加上各旁聊的未读（ADR-0093 §4）：话题里来了消息只动事件缓存，话题回执只动话题，SDK 都不当成
        // 房间的变化，单独跟着。
        let mut cache_changes = self
            .client
            .event_cache()
            .subscribe_to_room_generic_updates();
        let rooms_changed = self.rooms_changed.clone();
        // 初值是“还没算出来”，不是空列表：界面要分得清“还在加载”和“真的没有房间”。
        let (sender, receiver) = watch::channel(None);
        tokio::spawn(async move {
            let (entries, controller) = list.entries_with_dynamic_adapters(PAGE_SIZE);
            controller.set_filter(Box::new(filters::new_filter_non_left()));
            let mut entries = pin!(entries);
            let mut direct_changes = direct_changes.subscribe();
            let mut rooms = Vector::<RoomListItem>::new();
            loop {
                tokio::select! {
                    batch = entries.next() => {
                        let Some(batch) = batch else { return };
                        for diff in batch {
                            diff.apply(&mut rooms);
                        }
                    }
                    Some(_) = direct_changes.next() => {}
                    Ok(_) = cache_changes.recv() => {}
                    () = rooms_changed.notified() => {}
                    () = sender.closed() => return,
                }
                let mut infos = Vec::with_capacity(rooms.len());
                for room in &rooms {
                    infos.push(room_info(room).await);
                }
                if sender.send(Some(infos)).is_err() {
                    return;
                }
            }
        });
        Ok(snapshots(receiver))
    }
    pub fn refresh_rooms(&self) {
        self.rooms_changed.notify_one();
    }
}

fn snapshots<T: Clone + Send + Sync + 'static>(
    receiver: watch::Receiver<Option<T>>,
) -> impl Stream<Item = T> {
    stream::unfold((receiver, true), |(mut receiver, first)| async move {
        if !first {
            receiver.changed().await.ok()?;
        }
        loop {
            let value = receiver.borrow_and_update().clone();
            if let Some(value) = value {
                return Some((value, (receiver, false)));
            }
            receiver.changed().await.ok()?;
        }
    })
}

async fn room_info(room: &Room) -> RoomInfo {
    // 各字段隔着 await 取，成员状态可能在中间变化（比如接受邀请的加入在这时落地），拼出来的行会
    // 自相矛盾：`is_direct` 按受邀时邀请里的标记算，`direct_user_id` 却按加入后还没同步回来的
    // `m.direct` 算，得到「已加入、是私聊、不知道对方是谁」。以读到的状态为准，拼完核对，变了就按
    // 新状态重拼；成员状态变化本来就会让房间列表重发。
    let mut state = room.state();
    loop {
        let info = room_info_in(room, state).await;
        if room.state() == state {
            return info;
        }
        state = room.state();
    }
}

async fn room_info_in(room: &Room, state: RoomState) -> RoomInfo {
    // 私聊标记和对方从同一份数据取：好友邀请不一定能从邀请 stripped state 恢复 `is_direct`，但好友标记仍应进请求列表。
    let (is_direct, direct_user_id) = if state == RoomState::Invited {
        let friend = crate::friends::marked_friend(room).await;
        let is_direct = friend
            || room.is_direct().await.unwrap_or_else(|error| {
                tracing::warn!(room = %room.room_id(), "读不到私聊标记，按群聊显示：{error}");
                false
            });
        let peer = if is_direct {
            direct_peer(room).await
        } else {
            None
        };
        (is_direct, peer)
    } else {
        let targets = room.direct_targets();
        let is_direct = !targets.is_empty() || crate::friends::marked_friend(room).await;
        let peer = if !targets.is_empty() && targets.len() == 1 {
            targets
                .iter()
                .next()
                .and_then(|target| target.as_user_id().map(ToOwned::to_owned))
        } else {
            None
        };
        (is_direct, peer)
    };
    let mut name = match room.cached_display_name() {
        Some(name) => name.to_string(),
        None => room
            .display_name()
            .await
            .map_or_else(|_| room.room_id().to_string(), |name| name.to_string()),
    };
    // Tuwunel 的 Sliding Sync 不给房间的 heroes 带显示名，SDK 只能拿用户 ID 当私聊的名字；成员事件
    // 到得比显示名计算晚时，SDK 还会把 "Empty Room" 这类占位名缓存住，没有新的房间更新就不重算。
    // 这些占位名都不可读，去本机的成员事件里找对方的显示名（邀请里也有邀请人的成员事件）。
    if is_direct
        && (name.starts_with('@')
            || matches!(
                room.cached_display_name(),
                Some(RoomDisplayName::Empty | RoomDisplayName::EmptyWas(_))
            ))
        && let Some(peer) = peer_name(room).await
    {
        name = peer;
    }
    RoomInfo {
        room_id: room.room_id().to_owned(),
        name,
        is_direct,
        is_friend: crate::friends::marked_friend(room).await,
        direct_user_id,
        is_encrypted: room.encryption_state().is_encrypted(),
        membership: state.into(),
        latest: latest_message(room).await,
        unread_count: room.num_unread_messages() + crate::side_chat::threads_unread(room).await,
        unread_mentions: room.num_unread_mentions(),
        call_active: room.has_active_room_call(),
    }
}

async fn peer_name(room: &Room) -> Option<String> {
    let members = room
        .members_no_sync(RoomMemberships::JOIN | RoomMemberships::INVITE)
        .await
        .ok()?;
    let own = room.own_user_id();
    members
        .iter()
        .find(|member| member.user_id() != own)
        .and_then(|member| member.display_name().map(ToOwned::to_owned))
}

async fn latest_message(room: &Room) -> Option<LatestMessage> {
    let (timestamp, sender, is_own, profile, content) = match RoomExt::latest_event(room).await {
        LatestEventValue::Remote {
            timestamp,
            sender,
            is_own,
            profile,
            content,
        } => (timestamp, sender, is_own, profile, content),
        LatestEventValue::Local {
            timestamp,
            sender,
            profile,
            content,
            ..
        } => (timestamp, sender, true, profile, content),
        LatestEventValue::RemoteInvite { .. } | LatestEventValue::None => return None,
    };
    let sender_name = profile_name(&profile, &sender);
    Some(LatestMessage {
        sender_id: sender,
        sender_name,
        is_own,
        body: match &content {
            TimelineItemContent::MsgLike(message) => body(&message.kind),
            _ => MessageBody::Other {
                kind: "state".to_owned(),
            },
        },
        timestamp_ms: millis(timestamp),
        side_chat: crate::side_chat::latest_side_chat(room).await,
    })
}

fn profile_name(profile: &TimelineDetails<Profile>, sender: &OwnedUserId) -> String {
    match profile {
        TimelineDetails::Ready(Profile {
            display_name: Some(name),
            ..
        }) if !name.trim().is_empty() => name.clone(),
        _ => crate::message::fallback_name(sender).to_owned(),
    }
}

fn millis(timestamp: MilliSecondsSinceUnixEpoch) -> u64 {
    timestamp.get().into()
}

#[derive(Default)]
pub(crate) struct RoomSubscriptions {
    /// 打开着的房间。
    opened: Option<OwnedRoomId>,
    /// 桥等调用方显式打开、关闭的常驻 watch；重复打开不增加引用。
    watched: BTreeSet<OwnedRoomId>,
    /// 每份独立主对话或旁聊由 ChatRoom 的存活引用计数。
    focused: BTreeMap<OwnedRoomId, usize>,
    /// 加入后同步还标着“受邀”、临时订阅的房间，见 [`MatrixSession::until_joined`]。
    joining: BTreeSet<OwnedRoomId>,
    /// 上一次发给 SDK 的并集。名单没变就不发：SDK 收到订阅变化会取消在飞的同步请求，一次名单没变的
    /// 重发是无害的浪费，一次名单真变大的重发在 Tuwunel 上能把别的在飞请求顶成 `M_UNKNOWN_POS`（#370）。
    applied: BTreeSet<OwnedRoomId>,
}

pub(crate) struct RoomWatch {
    session: Arc<MatrixSession>,
    room_id: OwnedRoomId,
    runtime: tokio::runtime::Handle,
}

fn drop(&mut self) {
    let session = self.session.clone();
    let room_id = self.room_id.clone();
    self.runtime.spawn(async move {
        session.release_focused(&room_id).await;
    });
}

impl Drop for RoomWatch {
    fn drop(&mut self) {
        let session = self.session.clone();
        let room_id = self.room_id.clone();
        self.runtime.spawn(async move {
            session.release_focused(&room_id).await;
        });
    }
}

impl MatrixSession {
    pub async fn open_room(&self, room_id: &RoomId) -> Result<ChatRoom> {
        let room = self.subscribe_opened(room_id).await?;
        ChatRoom::open(room).await
    }
    pub(crate) async fn subscribe_opened(&self, room_id: &RoomId) -> Result<Room> {
        let room = self.joined(room_id)?;
        let mut subscriptions = self.subscriptions.lock().await;
        subscriptions.opened = Some(room_id.to_owned());
        self.subscribe(&mut subscriptions, false).await;
        Ok(room)
    }
    pub async fn watch_room(&self, room_id: &RoomId) -> Result<ChatRoom> {
        let room = self.joined(room_id)?;
        let mut subscriptions = self.subscriptions.lock().await;
        subscriptions.watched.insert(room_id.to_owned());
        self.subscribe(&mut subscriptions, false).await;
        drop(subscriptions);
        ChatRoom::open(room).await
    }
    pub(crate) async fn subscribe_watched(
        self: &Arc<Self>,
        room_id: &RoomId,
    ) -> Result<(Room, RoomWatch)> {
        let room = self.joined(room_id)?;
        let mut subscriptions = self.subscriptions.lock().await;
        *subscriptions.focused.entry(room_id.to_owned()).or_default() += 1;
        let watch = RoomWatch {
            session: self.clone(),
            room_id: room_id.to_owned(),
            runtime: tokio::runtime::Handle::current(),
        };
        self.subscribe(&mut subscriptions, false).await;
        Ok((room, watch))
    }
    pub async fn unwatch_room(&self, room_id: &RoomId) {
        let mut subscriptions = self.subscriptions.lock().await;
        subscriptions.watched.remove(room_id);
        self.subscribe(&mut subscriptions, false).await;
    }
    async fn release_focused(&self, room_id: &RoomId) {
        let mut subscriptions = self.subscriptions.lock().await;
        if let Some(count) = subscriptions.focused.get_mut(room_id) {
            *count -= 1;
            if *count == 0 {
                subscriptions.focused.remove(room_id);
            }
        }
        self.subscribe(&mut subscriptions, false).await;
    }
    pub(super) async fn subscribe(
        &self,
        subscriptions: &mut RoomSubscriptions,
        force_redeliver: bool,
    ) {
        let rooms: BTreeSet<OwnedRoomId> = subscriptions
            .opened
            .iter()
            .chain(&subscriptions.watched)
            .chain(subscriptions.focused.keys())
            .chain(&subscriptions.joining)
            .cloned()
            .collect();
        if rooms == subscriptions.applied && !force_redeliver {
            return;
        }
        let list: Vec<&RoomId> = rooms.iter().map(|room| &**room).collect();
        if force_redeliver {
            self.sync
                .room_list_service()
                .reset_and_add_room_subscriptions(&list)
                .await;
        } else {
            self.sync
                .room_list_service()
                .set_room_subscriptions(&list)
                .await;
        }
        subscriptions.applied = rooms;
    }
    pub(crate) async fn resubscribe(&self, subscriptions: &mut RoomSubscriptions) {
        subscriptions.applied.clear();
        self.subscribe(subscriptions, false).await;
    }
}

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

impl MatrixSession {
    pub async fn draft(&self, room_id: &RoomId, thread: Option<&EventId>) -> Result<Option<Draft>> {
        let room = self.joined(room_id)?;
        Ok(room.load_composer_draft(thread).await?.map(|draft| Draft {
            body: draft.plain_text,
            reply_to: match draft.draft_type {
                ComposerDraftType::Reply { event_id } => Some(event_id),
                ComposerDraftType::NewMessage | ComposerDraftType::Edit { .. } => None,
            },
        }))
    }
    pub async fn save_draft(
        &self,
        room_id: &RoomId,
        thread: Option<&EventId>,
        draft: &Draft,
    ) -> Result<()> {
        let room = self.joined(room_id)?;
        if draft.body.is_empty() && draft.reply_to.is_none() {
            room.clear_composer_draft(thread).await?;
        } else {
            let draft = ComposerDraft {
                plain_text: draft.body.clone(),
                // 不发格式化内容，正文就是原文。
                html_text: None,
                draft_type: match &draft.reply_to {
                    Some(event_id) => ComposerDraftType::Reply {
                        event_id: event_id.clone(),
                    },
                    None => ComposerDraftType::NewMessage,
                },
                attachments: Vec::new(),
            };
            room.save_composer_draft(draft, thread).await?;
        }
        Ok(())
    }
}

impl MatrixSession {
    pub(crate) fn joined(&self, room_id: &RoomId) -> Result<Room> {
        let room = self.room(room_id)?;
        if room.state() != RoomState::Joined {
            return Err(Error::NotReady("还没有加入这个聊天".to_owned()));
        }
        Ok(room)
    }
    pub(crate) fn room(&self, room_id: &RoomId) -> Result<Room> {
        self.client.get_room(room_id).ok_or_else(|| {
            Error::NotReady("这个聊天还没有同步到这台设备，稍等一会儿再试".to_owned())
        })
    }
}
