//! 房间列表：按最近活动排序，带名字、最新消息预览和未读数；建私聊、加入和离开房间。
//!
//! 列表由 matrix-sdk-ui 的 `RoomListService` 维护（和 Element X 一样），排序也用它的：正在发送的在前，
//! 其余按最近活动，再按名字。

use std::collections::BTreeSet;
use std::pin::pin;
use std::time::Duration;

use futures_util::{Stream, StreamExt, stream};
use matrix_sdk::{
    ComposerDraft, ComposerDraftType, Room, RoomDisplayName, RoomMemberships, RoomState,
    ruma::{
        MilliSecondsSinceUnixEpoch, OwnedRoomId, OwnedUserId, RoomId, UserId,
        api::{
            client::{
                membership::join_room_by_id,
                room::create_room::v3::{Request as CreateRoomRequest, RoomPreset},
            },
            error::ErrorKind,
        },
        events::{
            GlobalAccountDataEvent, direct::DirectEventContent,
            room::message::RoomMessageEventContent,
        },
    },
};
use matrix_sdk_ui::{
    eyeball_im::Vector,
    room_list_service::{RoomListItem, filters},
    timeline::{LatestEventValue, Profile, RoomExt, TimelineDetails, TimelineItemContent},
};
use tokio::sync::watch;

use crate::{ChatRoom, Draft, Error, MatrixSession, MessageBody, Result, message::body};

/// 房间列表一次最多展示的房间数。家里服务器的房间不多，一页就够。
const PAGE_SIZE: usize = 500;
/// 加入房间时等邀请事件落库的上限。邀请事件和房间在同一批同步里，等的只是这批写完。
const INVITE_WAIT: Duration = Duration::from_secs(10);
/// 发消息时等房间同步到本地的上限：传完文件发链接时，App 重启后同步刚开头；过了期限按没同步到/没加入报。
const ROOM_SYNC_WAIT: Duration = Duration::from_secs(20);
/// 加入后等同步把房间标成已加入的上限。加入事件会唤醒在等的同步，平时一秒内就到；超过这个时间按下面
/// [`MatrixSession::until_joined`] 说的 Tuwunel 的问题处理。
const JOIN_ECHO_WAIT: Duration = Duration::from_secs(3);
/// 重新订阅后等服务器发来完整状态的上限。
const RESUBSCRIBED_WAIT: Duration = Duration::from_secs(30);

/// 本账号在房间里的成员状态。
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

/// Sliding Sync 订阅完整时间线的房间。`opened`、`watched`、`joining` 是三份互不相干的需要，并集才是发给
/// 服务器的名单：只要还有一份要这个房间，它就留在名单里——退订的一方动不到别人还需要的房间，这就是
/// 引用计数（#370）。
#[derive(Default)]
pub(crate) struct RoomSubscriptions {
    /// 打开着的房间。
    opened: Option<OwnedRoomId>,
    /// 一直跟着的房间。
    watched: BTreeSet<OwnedRoomId>,
    /// 加入后同步还标着“受邀”、临时订阅的房间，见 [`MatrixSession::until_joined`]。
    joining: BTreeSet<OwnedRoomId>,
    /// 上一次发给 SDK 的并集。名单没变就不发：SDK 收到订阅变化会取消在飞的同步请求，一次名单没变的
    /// 重发是无害的浪费，一次名单真变大的重发在 Tuwunel 上能把别的在飞请求顶成 `M_UNKNOWN_POS`（#370）。
    applied: BTreeSet<OwnedRoomId>,
}

/// 房间列表里的一项。
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
}

/// 房间的最新一条消息。
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
}

impl MatrixSession {
    /// 和另一个用户建一个私聊房间并邀请对方，返回房间 ID。`encrypted` 决定是否开端到端加密。
    pub async fn create_direct_chat(&self, peer: &UserId, encrypted: bool) -> Result<OwnedRoomId> {
        // 建房时邀请失败，服务器只记日志：对方不存在时房间照样建出来，里面只有自己（Tuwunel 上实测）。
        if let Err(error) = self.client.account().fetch_user_profile_of(peer).await
            && error.client_api_error_kind() == Some(&ErrorKind::NotFound)
        {
            return Err(Error::InvalidInput(format!("找不到用户 {peer}")));
        }
        let room = if encrypted {
            self.client.create_dm(peer).await?
        } else {
            let mut request = CreateRoomRequest::new();
            request.invite = vec![peer.to_owned()];
            request.is_direct = true;
            request.preset = Some(RoomPreset::TrustedPrivateChat);
            self.client.create_room(request).await?
        };
        Ok(room.room_id().to_owned())
    }

    /// 已有的和 `peer` 的私聊（已加入或已受邀的）。
    pub fn direct_chat_with(&self, peer: &UserId) -> Option<OwnedRoomId> {
        self.client
            .get_dm_rooms(peer)
            .find(|room| matches!(room.state(), RoomState::Joined | RoomState::Invited))
            .map(|room| room.room_id().to_owned())
    }

    /// 现在在 `m.direct` 里标给 `peer` 的全部房间，照本机存的最新一份算，不看房间现在的成员状态。
    ///
    /// 认私聊以这份为准，别用房间列表的行认：行的私聊字段跟着每次列表重算重取，`m.direct` 还没同步到、
    /// 或 Tuwunel 补状态的间隙里，同一房间在相邻两次重算里一会儿算一会儿不算；`m.direct` 只在真的收到
    /// 事件时才变（#370）。也不按成员状态过滤（[`Self::direct_chat_with`] 那样只看已加入的不行）：补状态
    /// 的间隙里已加入的房间会被错标成受邀，按状态过滤就把这份稳定的事实也丢了；真离开的房间由“行从
    /// 列表消失”这条出集合的依据兜住。
    pub async fn direct_chat_rooms_with(&self, peer: &UserId) -> Result<Vec<OwnedRoomId>> {
        let raw = self
            .client
            .account()
            .account_data::<DirectEventContent>()
            .await
            .map_err(|error| Error::Upstream(error.to_string()))?;
        let Some(raw) = raw else {
            return Ok(Vec::new());
        };
        let event = raw
            .deserialize()
            .map_err(|error| Error::Upstream(error.to_string()))?;
        Ok(event
            .0
            .iter()
            .find(|(user, _)| user.as_user_id() == Some(peer))
            .map(|(_, rooms)| rooms.clone())
            .unwrap_or_default())
    }

    /// 解出房间链接的实际 ID，不改变成员关系；是否已加入由调用方按自己的导航边界决定。
    pub async fn resolve_room_id(&self, input: &str) -> Result<OwnedRoomId> {
        let target = <&matrix_sdk::ruma::RoomOrAliasId>::try_from(input)
            .map_err(|error| Error::InvalidInput(error.to_string()))?;
        match <&RoomId>::try_from(target) {
            Ok(id) => Ok(id.to_owned()),
            Err(alias) => Ok(self.client.resolve_room_alias(alias).await?.room_id),
        }
    }

    /// 加入房间，用于接受邀请。私聊邀请加入后记进 `m.direct`，之后列表里仍标为私聊。等同步把房间标成已加入才返回，
    /// 调用方接着就能往里发消息。
    pub async fn join_room(&self, room_id: &RoomId) -> Result<()> {
        // 私聊标记和邀请人要在加入前从邀请里读。不用 SDK 的 `join_room_by_id`：它加入后用 `set_is_direct`
        // 记私聊，拿本机存的 `m.direct` 改完整份写回，而本机这份要等写入同步回来才更新。一次同步之内先后接受
        // 两个私聊邀请时，后一次写的没有前一个房间，前一个就不再是私聊（通知房间因此能发消息，负载高时实测）。
        // `mark_as_dm` 先从服务器取最新的再改，同一会话里的写入有锁串着。代价是不导入邀请人随邀请分享的历史
        // 密钥：SDK 只在单独邀请时分享，建房时一起发的私聊邀请本来就没有。
        let invited = self
            .client
            .get_room(room_id)
            .filter(|room| room.state() == RoomState::Invited);
        // 好友邀请也记成私聊：它的邀请请求本身不带 `is_direct`，靠建房时的标记认。
        let inviter = match &invited {
            Some(room)
                if invited_as_direct(room).await || crate::friends::marked_friend(room).await =>
            {
                Some(room.invite_details().await?.inviter_id)
            }
            _ => None,
        };
        self.client
            .send(join_room_by_id::v3::Request::new(room_id.to_owned()))
            .await?;
        if let Some(inviter) = inviter {
            self.client
                .account()
                .mark_as_dm(room_id, &[inviter])
                .await?;
        }
        match invited {
            Some(room) => self.until_joined(&room).await,
            None => Ok(()),
        }
    }

    /// 等同步把刚加入的房间标成已加入。
    ///
    /// 绕过 Tuwunel 的问题（v1.9.4，<https://github.com/matrix-construct/tuwunel/issues/685>）：
    /// 它先让新事件对同步可见，再更新房间的当前状态。
    /// 这中间的一次 Sliding Sync，时间线里有加入事件，`required_state` 里本人却还是邀请；SDK 只按
    /// `required_state` 定状态，房间就一直是“受邀”，直到房间里来新事件。这时单独订阅这个房间，服务器按
    /// 当前状态重发。上游修好、镜像升级后删掉订阅这一步，只留等待。
    async fn until_joined(&self, room: &Room) -> Result<()> {
        if joined(room, JOIN_ECHO_WAIT).await {
            return Ok(());
        }
        tracing::warn!(room = %room.room_id(), "加入后同步仍标着受邀，重新订阅这个房间");
        let room_id = room.room_id().to_owned();
        let mut subscriptions = self.subscriptions.lock().await;
        subscriptions.joining.insert(room_id.clone());
        // 这次发送的目的就是让服务器按当前状态重发这个房间：房间多半还在名单里（跟着卡片），名单没变的
        // 下发 SDK 只当空操作，不会取消在飞的请求，服务器也就不会重发，所以强制。
        self.subscribe(&mut subscriptions, true).await;
        drop(subscriptions);
        let done = joined(room, RESUBSCRIBED_WAIT).await;
        let mut subscriptions = self.subscriptions.lock().await;
        subscriptions.joining.remove(&room_id);
        self.subscribe(&mut subscriptions, false).await;
        drop(subscriptions);
        if done {
            Ok(())
        } else {
            Err(Error::NotReady(
                "已经加入，但服务器一直没把这个聊天同步过来".to_owned(),
            ))
        }
    }

    pub async fn leave_room(&self, room_id: &RoomId) -> Result<()> {
        self.room(room_id)?.leave().await?;
        Ok(())
    }

    /// 房间列表流：第一项是算出来的第一份列表（SDK 设好过滤器后总会先给一份，没有房间时是空表），之后列表或
    /// 其中的房间变化时给出新列表；只含已加入和收到邀请的房间。
    ///
    /// 来不及取走的中间列表会被合并掉。要先 [`start_sync`](Self::start_sync) 才会有房间。
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

    /// 打开一个已加入的房间，返回可以收发消息的 [`ChatRoom`]。
    ///
    /// 房间列表只同步每个房间的最新一条消息；打开房间时才订阅它的完整时间线，和在手机上点进
    /// 一个聊天一致。同一时间只打开一个房间，打开另一个房间会取消前一个的订阅，[`watch_room`](Self::watch_room)
    /// 的房间不受影响。
    pub async fn open_room(&self, room_id: &RoomId) -> Result<ChatRoom> {
        let room = self.joined(room_id)?;
        let mut subscriptions = self.subscriptions.lock().await;
        subscriptions.opened = Some(room_id.to_owned());
        self.subscribe(&mut subscriptions, false).await;
        drop(subscriptions);
        ChatRoom::open(room).await
    }

    /// 一直跟着一个已加入的房间：不管打开着哪个房间，都同步它的完整时间线。不用了调
    /// [`unwatch_room`](Self::unwatch_room)。
    pub async fn watch_room(&self, room_id: &RoomId) -> Result<ChatRoom> {
        let room = self.joined(room_id)?;
        let mut subscriptions = self.subscriptions.lock().await;
        subscriptions.watched.insert(room_id.to_owned());
        self.subscribe(&mut subscriptions, false).await;
        drop(subscriptions);
        ChatRoom::open(room).await
    }

    pub async fn unwatch_room(&self, room_id: &RoomId) {
        let mut subscriptions = self.subscriptions.lock().await;
        subscriptions.watched.remove(room_id);
        self.subscribe(&mut subscriptions, false).await;
    }

    /// 调用方拿着锁等它结束：SDK 按最后一次的名单订阅，两次调用不能交错，否则后算的名单可能先生效。
    /// 名单没变就不发；`force_redeliver` 时即使没变也发，而且走 `reset_and_add`：SDK 会取消在飞的请求
    /// 重新发，服务器按当前状态重发房间。发送的目的只是改名单时用差集（`false`），要服务器重发状态时
    /// 才强制（`true`，比如 [`MatrixSession::until_joined`] 的重新订阅）。
    async fn subscribe(&self, subscriptions: &mut RoomSubscriptions, force_redeliver: bool) {
        let rooms: BTreeSet<OwnedRoomId> = subscriptions
            .opened
            .iter()
            .chain(&subscriptions.watched)
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

    /// 同步服务停止后再恢复名单；调用方持锁到重新启动，首条请求才能带上恢复后的订阅。
    pub(crate) async fn resubscribe(&self, subscriptions: &mut RoomSubscriptions) {
        subscriptions.applied.clear();
        self.subscribe(subscriptions, false).await;
    }

    /// 往已加入的房间发一条文字，不打开它：不动 Sliding Sync 的房间订阅，正在看的房间的时间线不受影响。不经发送队列，也没有本地
    /// 回显，发出去之后经同步出现；失败当场返回。房间还没同步到本地时等它一会儿：传完文件发链接时，App 重启后同步刚开头。
    pub async fn send_text_to(&self, room_id: &RoomId, body: &str) -> Result<()> {
        let room = self.joined_within(room_id, ROOM_SYNC_WAIT).await?;
        room.send(RoomMessageEventContent::text_plain(body)).await?;
        Ok(())
    }

    /// 房间同步到本地并成为已加入；等过了期限还没有，按原来的错报（没同步到，或者没加入）。
    async fn joined_within(&self, room_id: &RoomId, wait: Duration) -> Result<Room> {
        let deadline = tokio::time::Instant::now() + wait;
        loop {
            if let Some(room) = self.client.get_room(room_id)
                && room.state() == RoomState::Joined
            {
                return Ok(room);
            }
            if tokio::time::Instant::now() >= deadline {
                return self.joined(room_id);
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    /// 这个房间输入框里的草稿，没打过字也没在回复时是 `None`。存在本机状态库里，关掉 App 也还在；不上服务器。
    ///
    /// 和 [`open_room`](Self::open_room) 不同，这里不订阅房间：敲字的房间和打开着的房间不是同一个时不打断那边的时间线。
    pub async fn draft(&self, room_id: &RoomId) -> Result<Option<Draft>> {
        let room = self.joined(room_id)?;
        Ok(room.load_composer_draft(None).await?.map(|draft| Draft {
            body: draft.plain_text,
            reply_to: match draft.draft_type {
                ComposerDraftType::Reply { event_id } => Some(event_id),
                ComposerDraftType::NewMessage | ComposerDraftType::Edit { .. } => None,
            },
        }))
    }

    /// 记下草稿（matrix-sdk 的 composer draft，回复存成 `ComposerDraftType::Reply`），正文空、也没在回复时删掉。
    /// 存在本机状态库里。
    pub async fn save_draft(&self, room_id: &RoomId, draft: &Draft) -> Result<()> {
        let room = self.joined(room_id)?;
        if draft.body.is_empty() && draft.reply_to.is_none() {
            room.clear_composer_draft(None).await?;
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
            room.save_composer_draft(draft, None).await?;
        }
        Ok(())
    }

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

/// 把 watch 通道变成流：第一项是第一份算出来的值（还没算出来就等），之后每次变化给出最新值。
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

/// 在 `wait` 之内房间有没有变成已加入。
async fn joined(room: &Room, wait: Duration) -> bool {
    let mut updates = room.subscribe_info();
    let joined = async {
        while room.state() != RoomState::Joined {
            if updates.next().await.is_none() {
                return false;
            }
        }
        true
    };
    tokio::time::timeout(wait, joined).await.unwrap_or(false)
}

/// 邀请里有没有私聊标记。SDK 处理到一个新房间就把它放进列表、标成“受邀”，这批同步写完才读得到邀请事件；这之前
/// `is_direct` 答的“否”不算数，当真的话这个私聊再也不会进 `m.direct`（负载高时实测）。
async fn invited_as_direct(room: &Room) -> bool {
    let mut updates = room.subscribe_info();
    let invite_stored = async {
        loop {
            match room.get_member_no_sync(room.own_user_id()).await {
                Ok(None) => {}
                other => return other.map(|_| ()),
            }
            updates.next().await;
        }
    };
    let direct = match tokio::time::timeout(INVITE_WAIT, invite_stored).await {
        Ok(Ok(())) => room.is_direct().await.map_err(matrix_sdk::Error::from),
        Ok(Err(error)) => Err(error),
        Err(_) => {
            tracing::warn!(room = %room.room_id(), "等不到邀请事件，按群聊加入");
            Ok(false)
        }
    };
    direct.unwrap_or_else(|error| {
        tracing::warn!(room = %room.room_id(), "读不到邀请里的私聊标记：{error}");
        false
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

/// 按给定的成员状态拼一行；`room` 的实际状态变到别处时由 [`room_info`] 重拼。
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
        unread_count: room.num_unread_messages(),
    }
}

/// 受邀的房间还没进 `m.direct`，对方取邀请人。
async fn direct_peer(room: &Room) -> Option<OwnedUserId> {
    match room.invite_details().await {
        Ok(invite) => Some(invite.inviter_id),
        Err(error) => {
            tracing::warn!(room = %room.room_id(), "读不到邀请人：{error}");
            None
        }
    }
}

/// 私聊里对方的显示名；没有设置或者本机还没有对方的成员事件时为 `None`。
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
    })
}

fn profile_name(profile: &TimelineDetails<Profile>, sender: &OwnedUserId) -> String {
    match profile {
        TimelineDetails::Ready(Profile {
            display_name: Some(name),
            ..
        }) => name.clone(),
        _ => sender.to_string(),
    }
}

fn millis(timestamp: MilliSecondsSinceUnixEpoch) -> u64 {
    timestamp.get().into()
}
