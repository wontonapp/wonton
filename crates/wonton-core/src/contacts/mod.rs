//! Bot 的联系人（ADR-0045、ADR-0050）：同机和跨机同一份列表，存 `core/contacts.json`。
//!
//! - 归属人在网关的联系人接口发请求、同意、拒绝、删除（`gateway/contacts.rs`）；跨机 Bot 之间的请求和结果
//!   经两台核心账号的房间（`exchange.rs`，事件类型和核对在协议 `bot_contacts`）。
//! - 对方是远端的人时不经核心房间：Bot 发带 `app.wonton.bot_contact` 的私聊邀请，他接受邀请才算同意；
//!   接受与否由核心拿管理接口查房间成员（`refresh_person`）。
//! - 有 Active 远端联系人的 Bot 不给插件工具（总线按 [`Contacts::bot_with_active_remote`] 拒），
//!   Active 联系人进 Bot 的 `WONTON_MATRIX_ALLOWED_USERS`（[`BotDirectory::rebuild_bot`] 重建容器）。
//!
//! 核对规则（ADR-0050）：收到的请求/结果先核对发送者是已配对服务器的核心账号、两个用户 ID 的服务器名
//! 对得上，对不上整条忽略；结果按我方请求 ID 对上，对不上的忽略。

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use tokio::sync::Mutex as AsyncMutex;
use wonton_protocol::{
    BotContact, BotContacts, BotRef, ContactError, ContactEvent, ContactKind, ContactOutcome,
    ContactRequest, ContactRequestPush, ContactState, PeerStatus, PendingContactRequest,
    RemoteContact,
};

use crate::peers::{self, Peers};

pub mod exchange;
mod lifecycle;
pub(crate) mod store;

/// 联系人系统需要的配对信息。实现在 [`crate::peers::Peers`]，测试给假的，不拖起整条隧道。
pub trait PeerDirectory: Send + Sync {
    /// 这台服务器和本机配对着吗（只认已配对的记录）。
    fn paired(&self, server_name: &str) -> bool;
    /// 联系人行带的配对状态：没配对时为空（ADR-0050 状态表的“没配对”）。
    fn status(&self, server_name: &str) -> Option<PeerStatus>;
    /// 已配对的服务器名单，变化时更新。
    fn paired_names(&self) -> tokio::sync::watch::Receiver<std::collections::BTreeSet<String>>;
}

impl PeerDirectory for Peers {
    fn paired(&self, server_name: &str) -> bool {
        self.lookup(server_name).is_some()
    }

    fn status(&self, server_name: &str) -> Option<PeerStatus> {
        self.contact_peer_status(server_name)
    }

    fn paired_names(&self) -> tokio::sync::watch::Receiver<std::collections::BTreeSet<String>> {
        self.paired_names()
    }
}

/// 联系人系统的调用方需要的本机 Bot 信息。实现在 [`crate::plugin`]（PluginManager），用 attach 避免
/// 互相持有构造依赖。
pub trait BotDirectory: Send + Sync {
    /// 这个 Matrix 用户 ID 属于本机哪个 Bot；不属于任何 Bot（人或陌生账号）时为空。
    fn bot_of_user(&self, user_id: &str) -> Option<BotRef>;
    /// Bot 的 Matrix 用户 ID；Bot 不在（插件卸载了）时为空。
    fn user_of_bot(&self, bot: &BotRef) -> Option<String>;
    /// Bot 的 Matrix 用户名 localpart：核心代 Bot 登录发邀请用。
    fn bot_localpart(&self, bot: &BotRef) -> Option<String>;
    /// Bot 的显示名（ADR-0044 的改名），没设过时为空。
    fn display_name(&self, bot: &BotRef) -> Option<String>;
    /// Bot 的归属人显示名。
    fn keeper_display_name(&self, bot: &BotRef) -> Option<String>;
    /// Bot 的归属人用户名（私人 Bot）；共享 Bot 时为空（主人管）。
    fn keeper_username(&self, bot: &BotRef) -> Option<String>;
    /// 联系人名单变了：按现在的名单重建这个 Bot 的容器（`WONTON_MATRIX_ALLOWED_USERS`）。失败只记日志，
    /// 容器还在按旧名单跑，下次重建或重启会带上新名单。
    fn rebuild_bot(&self, bot: &BotRef);
}

/// 联系人请求的推送：发给管它的人的设备（协议 `ContactRequestPush`）。`keeper` 是私人 Bot 的归属人用户名；
/// 私人 Bot 的归属人没有登记设备时不回退推给主人（ADR-0045 没批推给主人看家人的联系人），请求照样进
/// “等你确认”；共享 Bot 归主人管，主人的设备是正路。
pub trait ContactPush: Send + Sync {
    fn contact_request(&self, keeper: Option<&str>, push: &ContactRequestPush);
}

pub struct Contacts {
    inner: Mutex<store::Stored>,
    path: PathBuf,
    peers: Arc<dyn PeerDirectory>,
    matrix: OnceLock<Arc<crate::matrix::Matrix>>,
    bots: OnceLock<Arc<dyn BotDirectory>>,
    push: OnceLock<Arc<dyn ContactPush>>,
    /// 房间和同步循环的互斥：建房、发事件、处理收到的邀请都串行在这把锁上，避免两边同时给一对服务器建房。
    exchange: AsyncMutex<()>,
    /// 联系人房间的建立进度：每台服务器的退避和最后一次失败，进联系人行的视图（不落盘，重启后重算）。
    room_state: Mutex<RoomBackoff>,
}

/// 建房/邀请失败后的按服务器指数退避。退避期内不碰 Tuwunel，调用方立刻失败、事件留在 outbox 里等下一轮。
struct RoomBackoff {
    attempts: BTreeMap<String, u32>,
    next_at: BTreeMap<String, tokio::time::Instant>,
    last_error: BTreeMap<String, ContactError>,
    first: Duration,
    max: Duration,
}

impl RoomBackoff {
    fn new(first: Duration, max: Duration) -> Self {
        Self {
            attempts: BTreeMap::new(),
            next_at: BTreeMap::new(),
            last_error: BTreeMap::new(),
            first,
            max,
        }
    }

    /// 现在能试吗。被动等待（字节序较大的一方等邀请）不算失败，不进退避。
    fn ready(&self, server: &str) -> bool {
        self.next_at
            .get(server)
            .is_none_or(|at| *at <= tokio::time::Instant::now())
    }

    fn note_failure(&mut self, server: &str, error: ContactError) {
        let attempts = self.attempts.entry(server.to_owned()).or_default();
        let delay = self.first * (1u32 << (*attempts).min(16));
        *attempts += 1;
        self.next_at.insert(
            server.to_owned(),
            tokio::time::Instant::now() + delay.min(self.max),
        );
        self.last_error.insert(server.to_owned(), error);
    }

    fn note_success(&mut self, server: &str) {
        self.attempts.remove(server);
        self.next_at.remove(server);
        self.last_error.remove(server);
    }
}

impl Contacts {
    pub fn open(
        path: impl Into<PathBuf>,
        peers: Arc<dyn PeerDirectory>,
    ) -> Result<Arc<Self>, std::io::Error> {
        Self::open_with_backoff(path, peers, Duration::from_secs(2), Duration::from_secs(30))
    }

    /// 退避的节奏可以换（测试用短退避），生产行为见 [`Contacts::open`]。
    pub(crate) fn open_with_backoff(
        path: impl Into<PathBuf>,
        peers: Arc<dyn PeerDirectory>,
        first: Duration,
        max: Duration,
    ) -> Result<Arc<Self>, std::io::Error> {
        let path = path.into();
        let stored = store::read(&path)?;
        Ok(Arc::new(Self {
            inner: Mutex::new(stored),
            path,
            peers,
            matrix: OnceLock::new(),
            bots: OnceLock::new(),
            push: OnceLock::new(),
            exchange: AsyncMutex::new(()),
            room_state: Mutex::new(RoomBackoff::new(first, max)),
        }))
    }

    /// 建房/邀请现在能不能试（按服务器退避）。
    pub(crate) fn room_ready(&self, server: &str) -> bool {
        self.room_state
            .lock()
            .expect("房间退避锁中毒")
            .ready(server)
    }

    pub(crate) fn note_room_failure(&self, server: &str, error: ContactError) {
        self.room_state
            .lock()
            .expect("房间退避锁中毒")
            .note_failure(server, error);
    }

    pub(crate) fn note_room_ok(&self, server: &str) {
        self.room_state
            .lock()
            .expect("房间退避锁中毒")
            .note_success(server);
    }

    /// 联系人行里如实显示的房间失败（没有就是通了或在被动等待）。
    pub(crate) fn room_error(&self, server: &str) -> Option<ContactError> {
        self.room_state
            .lock()
            .expect("房间退避锁中毒")
            .last_error
            .get(server)
            .cloned()
    }

    /// 事务式改动：在副本上改、落盘、成了才提交进内存。落盘失败时内存和磁盘都不变，
    /// 调用方拿到 Err——绝不出现“接口说失败、内存已经改了”的状态（ADR-0050 的失败即停）。
    pub(crate) fn commit<T>(
        &self,
        edit: impl FnOnce(&mut store::Stored) -> T,
    ) -> Result<T, ContactError> {
        let mut stored = self.lock().clone();
        let result = edit(&mut stored);
        self.save(&stored)?;
        *self.lock() = stored;
        Ok(result)
    }

    pub fn with_matrix(self: &Arc<Self>, matrix: Arc<crate::matrix::Matrix>) -> Arc<Self> {
        let _ = self.matrix.set(matrix);
        self.clone()
    }

    pub fn with_bots(self: &Arc<Self>, bots: Arc<dyn BotDirectory>) -> Arc<Self> {
        let _ = self.bots.set(bots);
        self.clone()
    }

    pub fn with_push(self: &Arc<Self>, push: Arc<dyn ContactPush>) -> Arc<Self> {
        let _ = self.push.set(push);
        self.clone()
    }

    /// 测试看一行原始记录（协议视图之外的字段也在）。
    #[cfg(test)]
    pub(crate) fn stored_row(&self, bot: &BotRef, user_id: &str) -> Option<store::StoredContact> {
        self.lock().row(bot, user_id)
    }

    /// 测试直接放一行（不走红室握手；总线工具门的测试用它造 Active 的远端联系人）。
    #[cfg(test)]
    pub(crate) fn seed_row_for_test(&self, bot: &BotRef, row: store::StoredContact) {
        self.lock().upsert(bot, row);
    }

    /// 测试看 outbox：先落意图再发的断言用。
    #[cfg(test)]
    pub(crate) fn stored_outbox(&self) -> Vec<store::OutboxEvent> {
        self.lock().outbox.clone()
    }

    pub(crate) fn lock(&self) -> std::sync::MutexGuard<'_, store::Stored> {
        self.inner.lock().expect("联系人表锁中毒")
    }

    pub(crate) fn save(&self, stored: &store::Stored) -> Result<(), ContactError> {
        store::write(&self.path, stored).map_err(|_| ContactError::Upstream)
    }

    pub(crate) fn matrix(&self) -> Option<&Arc<crate::matrix::Matrix>> {
        self.matrix.get()
    }

    /// 联系人记录的路径（同步令牌放同一个目录）。
    pub(crate) fn store_path(&self) -> &std::path::Path {
        &self.path
    }

    pub(crate) fn exchange_lock(&self) -> &AsyncMutex<()> {
        &self.exchange
    }

    pub(crate) fn bots(&self) -> Option<&Arc<dyn BotDirectory>> {
        self.bots.get()
    }

    pub(crate) fn peers(&self) -> &Arc<dyn PeerDirectory> {
        &self.peers
    }

    /// 一个 Bot 的联系人列表（ADR-0050）：先等这个 Bot 的归属人处理的，再是其他的，各自按时间排。
    pub fn view(&self, bot: &BotRef) -> BotContacts {
        let stored = self.lock();
        let mut rows = stored.rows(bot);
        rows.sort_by_key(|row| (row.state != ContactState::Incoming, row.updated_at_ms));
        BotContacts {
            contacts: rows
                .iter()
                .map(|row| self.contact_view(&stored, row))
                .collect(),
        }
    }

    fn contact_view(&self, _stored: &store::Stored, row: &store::StoredContact) -> BotContact {
        // 同机的联系人从本机记录里现取名字；远端的核心自报过，存下来的就是它报的。
        let (display_name, keeper_display_name) = match (self.bots.get(), row.remote.is_none()) {
            (Some(bots), true) => match bots.bot_of_user(&row.user_id) {
                Some(other) => (bots.display_name(&other), bots.keeper_display_name(&other)),
                None => (row.display_name.clone(), row.keeper_display_name.clone()),
            },
            _ => (row.display_name.clone(), row.keeper_display_name.clone()),
        };
        let remote = row.remote.as_ref().map(|server_name| RemoteContact {
            server_name: server_name.clone(),
            peer_status: self.peers.status(server_name),
            room_error: self.room_error(server_name),
        });
        BotContact {
            user_id: row.user_id.clone(),
            display_name,
            keeper_display_name,
            kind: row.kind,
            state: row.state,
            remote,
        }
    }

    /// “等你确认”列表：所有 Bot 的 Incoming 行，最早收到的在前。
    pub fn pending_requests(&self) -> Vec<PendingContactRequest> {
        let stored = self.lock();
        let mut pending = Vec::new();
        for (key, rows) in &stored.bots {
            let bot = store::parse_key(key);
            for row in rows
                .iter()
                .filter(|row| row.state == ContactState::Incoming)
            {
                pending.push(PendingContactRequest {
                    bot: bot.clone(),
                    contact: self.contact_view(&stored, row),
                    received_at_ms: row.updated_at_ms,
                });
            }
        }
        pending.sort_by_key(|request| request.received_at_ms);
        pending
    }

    /// 这个 Bot 有没有 Active 的远端联系人：总线按它不给插件工具（ADR-0050）。
    pub fn bot_with_active_remote(&self, bot: &BotRef) -> bool {
        let stored = self.lock();
        stored
            .rows(bot)
            .iter()
            .any(|row| row.state == ContactState::Active && row.remote.is_some())
    }

    /// 注进 Bot 容器的 `WONTON_MATRIX_ALLOWED_USERS` 里、归属人之外的：Active 联系人（同机和远端都要，
    /// ADR-0045“成立后”、ADR-0050“对方的用户 ID 加进本机 Bot 的名单”）。
    pub fn allowed_users_extra(&self, bot: &BotRef) -> Vec<String> {
        let stored = self.lock();
        stored
            .rows(bot)
            .iter()
            .filter(|row| row.state == ContactState::Active)
            .map(|row| row.user_id.clone())
            .collect()
    }

    /// 归属人发联系人请求（ADR-0050）。`kind` 由归属人选，核心不去对方那里查是哪种。
    pub async fn request(
        self: &Arc<Self>,
        bot: &BotRef,
        request: &ContactRequest,
    ) -> Result<BotContacts, ContactError> {
        let bots = self.bots.get().ok_or(ContactError::Upstream)?;
        let own = self.matrix.get().ok_or(ContactError::Upstream)?;
        // 是本机 Bot 才有联系人；用户 ID 要是完整的 Matrix ID，且不是这个 Bot 自己。
        let Some(bot_user) = bots.user_of_bot(bot) else {
            return Err(ContactError::BotNotFound);
        };
        let Some((_, target_server)) = split_user(&request.user_id) else {
            return Err(ContactError::InvalidUserId);
        };
        if request.user_id == bot_user {
            return Err(ContactError::InvalidUserId);
        }
        let own_server = own.server_name();
        let remote = (target_server != own_server).then(|| target_server.to_owned());
        // 对方家里的人（kind=Person）只跨机：同机的人归家庭系统管，不进联系人。
        if request.kind == ContactKind::Person && remote.is_none() {
            return Err(ContactError::InvalidUserId);
        }
        if let Some(server) = &remote
            && !self.peers.paired(server)
        {
            return Err(ContactError::NotPaired);
        }
        let request_id = crate::secret::random_token();
        match (request.kind, remote) {
            (ContactKind::Bot, None) => self.request_local_bot(bot, &bot_user, &request.user_id),
            // 跨机：Bot 走核心房间，人由 Bot 发私聊邀请；其他取值落到 Other。
            (kind, Some(server)) => {
                self.request_remote(bot, &bot_user, &request.user_id, kind, &server, request_id)
                    .await
            }
            // 同机的人归家庭系统管，不进联系人（ADR-0050）。
            (_, None) => Err(ContactError::InvalidUserId),
        }
    }

    /// 同机的 Bot 请求：不经过 Matrix，进对方的表等它的归属人批，这里一次做完。
    fn request_local_bot(
        self: &Arc<Self>,
        bot: &BotRef,
        bot_user: &str,
        target_user: &str,
    ) -> Result<BotContacts, ContactError> {
        let bots = self.bots.get().ok_or(ContactError::Upstream)?;
        let request_id = crate::secret::random_token();
        // 锁只包住表操作；await 都不在锁里（跨 await 的 std 锁不是 Send）。
        let mut stored = self.lock();
        if stored
            .row(bot, target_user)
            .is_some_and(|row| matches!(row.state, ContactState::Active | ContactState::Pending))
        {
            return Err(ContactError::AlreadyContact);
        }
        let Some(target) = bots.bot_of_user(target_user) else {
            return Err(ContactError::InvalidUserId);
        };
        if stored
            .row(&target, bot_user)
            .is_some_and(|row| matches!(row.state, ContactState::Active | ContactState::Pending))
        {
            return Err(ContactError::AlreadyContact);
        }
        let at = peers::now_ms();
        // 两边互相发了请求直接成立（同配对，ADR-0050）。
        if stored
            .row(&target, bot_user)
            .is_some_and(|row| row.state == ContactState::Pending)
        {
            establish_local(&mut stored, bot, target_user, &target, bot_user, at);
            self.save(&stored)?;
            drop(stored);
            self.rebuild(bot);
            self.rebuild(&target);
            return Ok(self.view(bot));
        }
        stored.upsert(
            &target,
            store::StoredContact {
                user_id: bot_user.to_owned(),
                kind: ContactKind::Bot,
                state: ContactState::Incoming,
                remote: None,
                display_name: None,
                keeper_display_name: None,
                request_id: Some(request_id.clone()),
                room: None,
                updated_at_ms: at,
            },
        );
        stored.upsert(
            bot,
            store::StoredContact {
                user_id: target_user.to_owned(),
                kind: ContactKind::Bot,
                state: ContactState::Pending,
                remote: None,
                display_name: None,
                keeper_display_name: None,
                request_id: Some(request_id),
                room: None,
                updated_at_ms: at,
            },
        );
        self.save(&stored)?;
        drop(stored);
        self.notify_request(bot, target_user, None, None);
        Ok(self.view(bot))
    }

    /// 跨机的请求：Bot 的事件经核心账号的房间送过去，人的邀请由 Bot 发。
    async fn request_remote(
        self: &Arc<Self>,
        bot: &BotRef,
        bot_user: &str,
        target_user: &str,
        kind: ContactKind,
        server: &str,
        request_id: String,
    ) -> Result<BotContacts, ContactError> {
        let bots = self.bots.get().ok_or(ContactError::Upstream)?;
        if self
            .lock()
            .row(bot, target_user)
            .is_some_and(|row| matches!(row.state, ContactState::Active | ContactState::Pending))
        {
            return Err(ContactError::AlreadyContact);
        }
        match kind {
            // 跨机的 Bot：意图（Pending 行 + 出站事件）先落盘，送不送得到由收件循环按退避重试；
            // 接口返回的列表和本地状态一致，发送失败不回头改状态。
            ContactKind::Bot => {
                let event = ContactEvent::Request {
                    id: request_id.clone(),
                    from: bot_user.to_owned(),
                    to: target_user.to_owned(),
                    from_display_name: bots.display_name(bot),
                    keeper_display_name: bots.keeper_display_name(bot),
                };
                self.commit(|stored| {
                    stored.upsert(
                        bot,
                        store::StoredContact {
                            user_id: target_user.to_owned(),
                            kind: ContactKind::Bot,
                            state: ContactState::Pending,
                            remote: Some(server.to_owned()),
                            display_name: None,
                            keeper_display_name: None,
                            request_id: Some(request_id.clone()),
                            room: None,
                            updated_at_ms: peers::now_ms(),
                        },
                    );
                    stored.enqueue(server, event, request_id.clone(), peers::now_ms());
                })?;
                exchange::drain(self, server).await;
                Ok(self.view(bot))
            }
            // 对方家里的人：Bot 发私聊邀请，他本人接受（ADR-0050“对象是对方家里的人”）。邀请有确定
            // 结果（发出、被拒、查无此人），落成行的状态再提交；发送失败（传输）不落任何东西。
            ContactKind::Person => {
                let invited = exchange::invite_person(self, bot, target_user, server).await?;
                // 对方拒绝（MSC4155）或查无此人：记下这行，壳按状态写“对方拒绝了”“没有这个人”。
                let (state, room) = match invited {
                    exchange::Invited::Sent(room) => (ContactState::Pending, Some(room)),
                    exchange::Invited::Declined => (ContactState::Declined, None),
                    exchange::Invited::Missing => (ContactState::NotFound, None),
                };
                self.commit(|stored| {
                    stored.upsert(
                        bot,
                        store::StoredContact {
                            user_id: target_user.to_owned(),
                            kind: ContactKind::Person,
                            state,
                            remote: Some(server.to_owned()),
                            display_name: None,
                            keeper_display_name: None,
                            request_id: None,
                            room,
                            updated_at_ms: peers::now_ms(),
                        },
                    );
                })?;
                Ok(self.view(bot))
            }
            ContactKind::Other => Err(ContactError::InvalidUserId),
        }
    }

    /// 归属人同意一条进来的请求。
    /// 取列表前核对远端的人接没接受邀请：查私聊房间的成员（Tuwunel 的管理接口，2026-11-12 对 v1.9.4 实测
    /// 该接口对已登录用户开放）。对方接受了，这行落成 Active，Bot 的名单随之重建。
    pub async fn refresh_person(self: &Arc<Self>, bot: &BotRef) -> Result<(), ContactError> {
        let pending: Vec<(String, String)> = {
            let stored = self.lock();
            stored
                .rows(bot)
                .iter()
                .filter(|row| row.kind == ContactKind::Person && row.state == ContactState::Pending)
                .filter_map(|row| row.room.clone().map(|room| (row.user_id.clone(), room)))
                .collect()
        };
        for (user_id, room) in pending {
            let joined = exchange::room_contains(self, &room, &user_id).await?;
            if joined {
                let mut stored = self.lock();
                if stored.person_joined(bot, &user_id, peers::now_ms()).is_ok() {
                    self.save(&stored)?;
                    drop(stored);
                    self.rebuild(bot);
                }
            }
        }
        Ok(())
    }

    /// 远端经房间发来的事件已经核对过发送者和服务器名（`exchange.rs`），这里落到状态机。
    /// 本机 Bot 由事件的 `to` 认定（调用方解析好了）。返回要不要往回送结果（两边互相发了请求时送同意）。
    /// 落盘失败返回 Err：收件循环不推进同步令牌，事件会重投，处理是幂等的。
    pub(crate) fn on_remote_request(
        self: &Arc<Self>,
        bot: &BotRef,
        from_user: &str,
        from_server: &str,
        id: &str,
        display_name: Option<String>,
        keeper_display_name: Option<String>,
    ) -> Result<Option<ContactEvent>, ContactError> {
        let at = peers::now_ms();
        let taken = self.commit(|stored| {
            stored.on_remote_request(
                &store::RemoteRequest {
                    bot,
                    from_user,
                    from_server,
                    request_id: id,
                    display_name: display_name.clone(),
                    keeper_display_name: keeper_display_name.clone(),
                },
                at,
            )
        })?;
        match taken {
            store::Take::Recorded => {
                self.notify_request(bot, from_user, display_name, keeper_display_name);
                Ok(None)
            }
            store::Take::Established => {
                self.rebuild(bot);
                let bots = self.bots.get().ok_or(ContactError::Upstream)?;
                let bot_user = bots.user_of_bot(bot).ok_or(ContactError::BotNotFound)?;
                Ok(Some(ContactEvent::Result {
                    id: id.to_owned(),
                    from: bot_user,
                    to: from_user.to_owned(),
                    outcome: ContactOutcome::Approved,
                    keeper_display_name: bots.keeper_display_name(bot),
                }))
            }
            store::Take::Ignore => Ok(None),
        }
    }

    /// 远端发来的结果：事件里的 `to` 认定本机 Bot，行按发起方用户、服务器和请求 ID 认。
    /// 同意了要重建 Bot 的名单。落盘失败返回 Err，事件重投。
    pub(crate) fn on_remote_result(
        self: &Arc<Self>,
        bot: &BotRef,
        from_user: &str,
        from_server: &str,
        id: &str,
        outcome: ContactOutcome,
        keeper_display_name: Option<String>,
    ) -> Result<(), ContactError> {
        let at = peers::now_ms();
        let state = self.commit(|stored| {
            stored.on_remote_result(
                &store::RemoteResult {
                    bot,
                    from_user,
                    from_server,
                    request_id: id,
                    outcome,
                    keeper_display_name,
                },
                at,
            )
        })?;
        if state == Some(ContactState::Active) {
            self.rebuild(bot);
        }
        Ok(())
    }

    /// 远端删了联系人或取消了请求。删掉的是 Active 行时重建 Bot 的名单。落盘失败返回 Err，事件重投。
    pub(crate) fn on_remote_remove(
        self: &Arc<Self>,
        bot: &BotRef,
        from_user: &str,
        from_server: &str,
    ) -> Result<(), ContactError> {
        let removed = self.commit(|stored| stored.on_remote_remove(bot, from_user, from_server))?;
        if removed.is_some_and(|row| row.state == ContactState::Active) {
            self.rebuild(bot);
        }
        Ok(())
    }

    /// 记下核心联系人房间（对方服务器名 → 房间 ID）并落盘。
    pub(crate) fn set_peer_room(&self, server_name: &str, room: &str) -> Result<(), ContactError> {
        self.commit(|stored| stored.set_peer_room(server_name, room))?;
        Ok(())
    }

    pub(crate) fn peer_room(&self, server_name: &str) -> Option<String> {
        self.lock().peer_room(server_name)
    }

    /// 新请求推给归属人的设备，放进“等你确认”（ADR-0045、ADR-0050）。请求方的名字：同机的从本机记录里取，
    /// 远端的自报值随请求来。
    fn notify_request(
        &self,
        bot: &BotRef,
        user_id: &str,
        display_name: Option<String>,
        keeper: Option<String>,
    ) {
        let Some(push) = self.push.get() else {
            return;
        };
        let (display_name, keeper) = match self.bots.get() {
            Some(bots) if display_name.is_none() => match bots.bot_of_user(user_id) {
                Some(other) => (bots.display_name(&other), bots.keeper_display_name(&other)),
                None => (display_name, keeper),
            },
            _ => (display_name, keeper),
        };
        let keeper_username = self.bots.get().and_then(|bots| bots.keeper_username(bot));
        push.contact_request(
            keeper_username.as_deref(),
            &ContactRequestPush {
                bot: bot.clone(),
                user_id: user_id.to_owned(),
                display_name,
                keeper_display_name: keeper,
            },
        );
    }

    /// 联系人名单变了：重建 Bot 的容器让新名单生效。没接 Bot 目录（测试）时跳过。
    pub(crate) fn rebuild(self: &Arc<Self>, bot: &BotRef) {
        if let Some(bots) = self.bots.get() {
            bots.rebuild_bot(bot);
        }
    }
}

/// 同机的两边互相发了请求：两行一起落成 Active。
fn establish_local(
    stored: &mut store::Stored,
    requester: &BotRef,
    requester_user: &str,
    target: &BotRef,
    target_user: &str,
    at: u64,
) {
    for (bot, user) in [(requester, target_user), (target, requester_user)] {
        if let Some(mut row) = stored.row(bot, user) {
            row.state = ContactState::Active;
            row.updated_at_ms = at;
            stored.upsert(bot, row);
        }
    }
}

/// `@localpart:server` 的两半；不是完整用户 ID 时为空。
pub(crate) fn split_user(user_id: &str) -> Option<(String, String)> {
    let rest = user_id.strip_prefix('@')?;
    let (localpart, server) = rest.split_once(':')?;
    if localpart.is_empty() || server.is_empty() {
        return None;
    }
    Some((localpart.to_owned(), server.to_owned()))
}

#[cfg(test)]
#[path = "contacts_tests.rs"]
mod tests;

#[cfg(test)]
pub(crate) mod test_support;

#[cfg(test)]
#[path = "federation_docker_tests.rs"]
mod federation_docker_tests;
