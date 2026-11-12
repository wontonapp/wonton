//! 好友页（ADR-0051 b 路）：请求就是 Matrix 的私聊邀请，好友名单由核心存着。
//!
//! 好友关系是 Matrix 事实，但“谁是好友、在哪个组”要跨设备、给网关分享认人，所以名单在核心：
//! 有自家服务器时经 `/_wonton/api/friends` 取和改；外部 Matrix 账号没有服务器，清单放在本机
//! （[`local`]）。邀请、同意、拒绝、取消都直接走 Matrix，核心只在同意之后记下身份。
//!
//! 屏蔽分两种：个人屏蔽写 MSC4155 的 `m.ignored_user_list`（用户或整台服务器，只影响邀请），
//! 整台服务器拉黑写核心的 `forbidden_remote_server_names`，只有主人能做。

mod api;
mod local;

use api::Failure;

use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use futures_util::StreamExt;
use tokio::runtime::Handle;
use tokio::sync::watch;
use tokio::sync::watch::Sender as WatchSender;
use url::Url;
use wonton_protocol::{
    Action, CoreError, FriendBlock, FriendBlockScope, FriendBlockTarget, FriendFailure,
    FriendProblem, FriendRequest, FriendRequestKind, FriendRow, FriendsList, FriendsState,
    FriendsViewModel, MatrixTarget, ProxySettings,
};

use crate::chat::{ChatLink, matrix_target};
use crate::home::HomeLink;
use crate::l10n::text;
use crate::scope::reuse;

pub(crate) struct FriendsLink {
    handle: Handle,
    home: Arc<HomeLink>,
    chat: Arc<ChatLink>,
    dir: PathBuf,
    view: WatchSender<FriendsViewModel>,
    /// 和服务器说话用的本地代理客户端，随代理设置重换。
    client: Mutex<Option<(ProxySettings, reqwest::Client)>>,
    /// 同一时刻只跑一次取列表。
    queue: tokio::sync::Mutex<()>,
    /// 外部账号的本机清单。
    local: Mutex<local::Local>,
    /// 取到过列表：设置首页的入口要显示人数。
    listed: AtomicBool,
    /// 核心名单是不是主人的；家人（与外部账号一样）只留本机清单。
    server_scope: AtomicBool,
}

impl FriendsLink {
    pub(crate) fn new(
        handle: Handle,
        home: Arc<HomeLink>,
        chat: Arc<ChatLink>,
        dir: &Path,
        view: Option<WatchSender<FriendsViewModel>>,
    ) -> Result<Arc<Self>, CoreError> {
        let local = local::Local::open(dir).map_err(|error| CoreError::Storage {
            reason: text(
                "friends-local-open",
                &[("reason", error.to_string().into())],
            ),
        })?;
        let initial = FriendsViewModel {
            state: FriendsState::Loading,
            friends: Vec::new(),
            groups: Vec::new(),
            incoming: Vec::new(),
            outgoing: Vec::new(),
            blocked: Vec::new(),
            can_block_federation: false,
            my_link: None,
        };
        Ok(Arc::new(Self {
            handle,
            home,
            chat,
            dir: dir.to_owned(),
            view: reuse(view, initial),
            client: Mutex::new(None),
            queue: tokio::sync::Mutex::default(),
            local: Mutex::new(local),
            listed: AtomicBool::new(false),
            server_scope: AtomicBool::new(true),
        }))
    }

    pub(crate) fn sender(&self) -> WatchSender<FriendsViewModel> {
        self.view.clone()
    }

    pub(crate) fn subscribe(&self) -> watch::Receiver<FriendsViewModel> {
        self.view.subscribe()
    }

    pub(crate) fn dispatch(self: &Arc<Self>, action: Action) -> Result<(), CoreError> {
        match action {
            Action::RefreshFriends => self.refresh(true),
            Action::AddFriend { user_id } => self.add(user_id),
            Action::ApproveFriendRequest { room_id } => self.approve(room_id),
            Action::RejectFriendRequest { room_id } => self.reject(room_id),
            Action::CancelFriendRequest { room_id } => self.cancel(room_id),
            Action::RemoveFriend { user_id } => self.remove(user_id),
            Action::BlockFriend { target, scope } => self.block(target, scope),
            Action::UnblockFriend { target, scope } => self.unblock(target, scope),
            Action::SaveFriendGroup { id, name } => self.save_group(id, name),
            Action::RemoveFriendGroup { id } => self.remove_group(id),
            Action::SetFriendGroups { user_id, groups } => self.set_groups(user_id, groups),
            _ => return Err(invalid("friends-invalid-action")),
        }
        Ok(())
    }

    fn refresh(self: &Arc<Self>, mark_loading: bool) {
        let link = self.clone();
        self.spawn(async move { link.fetch(mark_loading).await });
    }

    /// 取当前账号的完整状态。有自家服务器时名单在服务器，外部账号在本机。
    async fn fetch(self: Arc<Self>, mark_loading: bool) {
        let _guard = self.queue.lock().await;
        if mark_loading {
            self.view
                .send_modify(|view| view.state = FriendsState::Loading);
        }
        let outcome = self.assemble().await;
        match outcome {
            Ok(view) => {
                self.listed.store(true, Ordering::SeqCst);
                self.view.send_modify(|current| *current = view);
            }
            Err(problem) => {
                tracing::debug!(reason = %problem.message, "friends list unavailable");
                self.view.send_modify(|view| {
                    view.state = FriendsState::Failed {
                        problem: problem.problem,
                    }
                });
            }
        }
    }

    async fn assemble(&self) -> Result<FriendsViewModel, Failure> {
        // 已知隧道不通就立刻失败，不等 Matrix 扫描（它经代理且默认无限重试）和 30 秒超时。
        if let Some(reason) = self.home.tunnel_outage() {
            return Err(Failure::transport(reason));
        }
        let session = self
            .chat
            .current()
            .map_err(|_| Failure::unauthorized("friends-not-signed-in"))?;
        let mut sync = collect(&session).await?;
        let ignored = session.blocked_list().await.map_err(Failure::from_matrix)?;

        tracing::debug!(
            server = self.uses_server(),
            joined = sync.joined.len(),
            "friends assemble"
        );
        let mut friends = Vec::new();
        let mut federation_blocked = Vec::new();
        let mut can_block_federation = false;
        let groups;
        if self.uses_server() {
            let view = self.server_view(&sync.joined).await?;
            groups = view.groups;
            can_block_federation = view.can_block_federation;
            federation_blocked = view.blocked_servers;
            for row in view.friends {
                friends.push(FriendRow {
                    display_name: row
                        .display_name
                        .or_else(|| name_of(&sync.joined, &row.user_id)),
                    ..row
                });
            }
        } else {
            let local = self.local.lock().expect("friends local lock");
            groups = local.groups.clone();
            for (user_id, name, invited_at_ms) in sync.joined.drain(..) {
                if local
                    .removed
                    .get(&user_id)
                    .is_some_and(|removed_at_ms| invited_at_ms <= *removed_at_ms)
                {
                    continue;
                }
                friends.push(FriendRow {
                    display_name: Some(name),
                    groups: local.groups_of(&user_id),
                    user_id,
                    shared_plugins: Vec::new(),
                    last_opened_share_at_ms: None,
                });
            }
        }
        friends.sort_by(|a, b| a.user_id.cmp(&b.user_id));
        sync.incoming
            .sort_by_key(|request| std::cmp::Reverse(request.invited_at_ms));
        sync.outgoing
            .sort_by_key(|request| std::cmp::Reverse(request.invited_at_ms));

        let mut blocked = blocked_from(ignored);
        if can_block_federation {
            for server in federation_blocked {
                blocked.push(FriendBlock {
                    target: FriendBlockTarget::Server {
                        server_name: server,
                    },
                    scope: FriendBlockScope::Federation,
                    blocked_at_ms: 0,
                });
            }
        }
        Ok(FriendsViewModel {
            state: FriendsState::Ready,
            friends,
            groups,
            incoming: sync.incoming,
            outgoing: sync.outgoing,
            blocked,
            can_block_federation,
            my_link: Some(format!("https://matrix.to/#/{}", session.user_id())),
        })
    }

    /// 名单在服务器上时：取列表，并把同意过但没记上的补记（删掉的墓碑不补）。
    async fn server_view(&self, joined: &[(String, String, u64)]) -> Result<FriendsList, Failure> {
        let first = self.api()?.list().await?;
        self.server_scope.store(first.owner_scope, Ordering::SeqCst);
        tracing::debug!(
            friends = first.friends.len(),
            removed = first.removed.len(),
            joined = joined.len(),
            "friends server view"
        );
        for (user_id, name, invited_at_ms) in joined {
            let still_removed = first
                .removed
                .iter()
                .find(|removed| &removed.user_id == user_id)
                .is_some_and(|removed| *invited_at_ms <= removed.removed_at_ms);
            if !first.friends.iter().any(|row| &row.user_id == user_id) && !still_removed {
                self.api()?.accept(user_id, Some(name)).await?;
            }
        }
        if joined.is_empty() {
            Ok(first)
        } else {
            let again = self.api()?.list().await?;
            tracing::debug!(friends = again.friends.len(), "friends server view again");
            Ok(again)
        }
    }

    fn add(self: &Arc<Self>, user_id: String) {
        let Some(user_id) = friend_user_id(&user_id) else {
            self.view.send_modify(|view| {
                view.state = FriendsState::Failed {
                    problem: FriendProblem {
                        reason: FriendFailure::InvalidUserId,
                        http_status: None,
                        matrix_errcode: None,
                        retry_after_ms: None,
                    },
                }
            });
            return;
        };
        let link = self.clone();
        self.spawn(async move {
            link.view
                .send_modify(|view| view.state = FriendsState::Loading);
            let session = match link.chat.current() {
                Ok(session) => session,
                Err(_) => return link.fail_fixed(FriendFailure::Unauthorized),
            };
            let parsed = match wonton_matrix::OwnedUserId::try_from(user_id.as_str()) {
                Ok(parsed) => parsed,
                Err(_) => return link.fail_fixed(FriendFailure::InvalidUserId),
            };
            match session.invite_friend(&parsed).await {
                Ok(_) => link.refresh(true),
                Err(error) => link.fail(error),
            }
        });
    }

    fn approve(self: &Arc<Self>, room_id: String) {
        let link = self.clone();
        self.spawn(async move {
            link.view
                .send_modify(|view| view.state = FriendsState::Loading);
            let session = match link.chat.current() {
                Ok(session) => session,
                Err(_) => return link.fail_fixed(FriendFailure::Unauthorized),
            };
            let Ok(room) = wonton_matrix::OwnedRoomId::try_from(room_id.as_str()) else {
                return link.fail_fixed(FriendFailure::RequestNotFound);
            };
            let invite = session.friend_invite(&room).await.ok().flatten();
            if let Err(error) = session.accept_invite(&room).await {
                return link.fail(error);
            }
            if let Some(invite) = invite.filter(|invite| invite.friend)
                && link.uses_server()
                && let Ok(api) = link.api()
                && let Err(error) = api
                    .accept(invite.inviter.as_str(), invite.inviter_name.as_deref())
                    .await
            {
                return link.fail_api(error);
            }
            link.refresh(true);
        });
    }

    fn reject(self: &Arc<Self>, room_id: String) {
        self.request(room_id, |session, room| async move {
            session.reject_invite(&room).await
        });
    }

    fn cancel(self: &Arc<Self>, room_id: String) {
        self.request(room_id, |session, room| async move {
            session.cancel_friend_invite(&room).await
        });
    }

    fn request<F, Fut>(self: &Arc<Self>, room_id: String, run: F)
    where
        F: FnOnce(Arc<wonton_matrix::MatrixSession>, wonton_matrix::OwnedRoomId) -> Fut
            + Send
            + 'static,
        Fut: Future<Output = Result<(), wonton_matrix::Error>> + Send,
    {
        let link = self.clone();
        self.spawn(async move {
            link.view
                .send_modify(|view| view.state = FriendsState::Loading);
            let session = match link.chat.current() {
                Ok(session) => session,
                Err(_) => return link.fail_fixed(FriendFailure::Unauthorized),
            };
            let Ok(room) = wonton_matrix::OwnedRoomId::try_from(room_id.as_str()) else {
                return link.fail_fixed(FriendFailure::RequestNotFound);
            };
            match run(session, room).await {
                Ok(()) => link.refresh(true),
                Err(error) => link.fail(error),
            }
        });
    }

    fn remove(self: &Arc<Self>, user_id: String) {
        let link = self.clone();
        self.spawn(async move {
            if link.uses_server() {
                let api = match link.api() {
                    Ok(api) => api,
                    Err(failure) => return link.fail_api(failure),
                };
                match api.remove(&user_id).await {
                    Ok(()) => link.refresh(true),
                    Err(failure) => link.fail_api(failure),
                }
            } else {
                {
                    let mut local = link.local.lock().expect("friends local lock");
                    local.removed.insert(user_id.clone(), now_ms());
                    local.assigned.remove(&user_id);
                    if let Err(error) = local.save(&link.dir) {
                        return link.fail_text(FriendFailure::Other, error.to_string());
                    }
                }
                link.refresh(true);
            }
        });
    }

    fn block(self: &Arc<Self>, target: FriendBlockTarget, scope: FriendBlockScope) {
        self.blocking(target, scope, true);
    }

    fn unblock(self: &Arc<Self>, target: FriendBlockTarget, scope: FriendBlockScope) {
        self.blocking(target, scope, false);
    }

    fn blocking(
        self: &Arc<Self>,
        target: FriendBlockTarget,
        scope: FriendBlockScope,
        blocked: bool,
    ) {
        let link = self.clone();
        self.spawn(async move {
            match (scope, &target) {
                (FriendBlockScope::Invitations, _) => {
                    let session = match link.chat.current() {
                        Ok(session) => session,
                        Err(_) => return link.fail_fixed(FriendFailure::Unauthorized),
                    };
                    let mut list = match session.blocked_list().await {
                        Ok(list) => list,
                        Err(error) => return link.fail(error),
                    };
                    match &target {
                        FriendBlockTarget::User { user_id } => match user_id.parse() {
                            Ok(id) if blocked => {
                                list.users.insert(id);
                            }
                            Ok(id) => {
                                list.users.remove(&id);
                            }
                            Err(_) => return link.fail_fixed(FriendFailure::InvalidUserId),
                        },
                        FriendBlockTarget::Server { server_name } => match server_name.parse() {
                            Ok(name) if blocked => {
                                list.servers.insert(name);
                            }
                            Ok(name) => {
                                list.servers.remove(&name);
                            }
                            Err(_) => return link.fail_fixed(FriendFailure::InvalidUserId),
                        },
                        FriendBlockTarget::Other => {
                            return link.fail_fixed(FriendFailure::Other);
                        }
                    }
                    match session.set_blocked_list(&list).await {
                        Ok(()) => link.refresh(true),
                        Err(error) => link.fail(error),
                    }
                }
                (FriendBlockScope::Federation, FriendBlockTarget::Server { server_name }) => {
                    let api = match link.api() {
                        Ok(api) => api,
                        Err(failure) => return link.fail_api(failure),
                    };
                    match api.set_block(server_name, blocked).await {
                        Ok(()) => link.refresh(true),
                        Err(failure) => link.fail_api(failure),
                    }
                }
                (FriendBlockScope::Federation, _) => link.fail_fixed(FriendFailure::Other),
                (FriendBlockScope::Other, _) => link.fail_fixed(FriendFailure::Other),
            }
        });
    }

    fn save_group(self: &Arc<Self>, id: Option<String>, name: String) {
        let link = self.clone();
        self.spawn(async move {
            if link.uses_server() {
                let api = match link.api() {
                    Ok(api) => api,
                    Err(failure) => return link.fail_api(failure),
                };
                match api.save_group(id.as_deref(), &name).await {
                    Ok(_) => link.refresh(true),
                    Err(failure) => link.fail_api(failure),
                }
            } else {
                {
                    let mut local = link.local.lock().expect("friends local lock");
                    if let Err(error) = local.save_group(id.as_deref(), &name) {
                        return link.fail_text(FriendFailure::Other, error);
                    }
                    if let Err(error) = local.save(&link.dir) {
                        return link.fail_text(FriendFailure::Other, error.to_string());
                    }
                }
                link.refresh(true);
            }
        });
    }

    fn remove_group(self: &Arc<Self>, id: String) {
        let link = self.clone();
        self.spawn(async move {
            if link.uses_server() {
                let api = match link.api() {
                    Ok(api) => api,
                    Err(failure) => return link.fail_api(failure),
                };
                match api.remove_group(&id).await {
                    Ok(()) => link.refresh(true),
                    Err(failure) => link.fail_api(failure),
                }
            } else {
                {
                    let mut local = link.local.lock().expect("friends local lock");
                    local.remove_group(&id);
                    if let Err(error) = local.save(&link.dir) {
                        return link.fail_text(FriendFailure::Other, error.to_string());
                    }
                }
                link.refresh(true);
            }
        });
    }

    fn set_groups(self: &Arc<Self>, user_id: String, groups: Vec<String>) {
        let link = self.clone();
        self.spawn(async move {
            if link.uses_server() {
                let api = match link.api() {
                    Ok(api) => api,
                    Err(failure) => return link.fail_api(failure),
                };
                match api.set_groups(&user_id, &groups).await {
                    Ok(()) => link.refresh(true),
                    Err(failure) => link.fail_api(failure),
                }
            } else {
                {
                    let mut local = link.local.lock().expect("friends local lock");
                    local.set_groups(&user_id, &groups);
                    if let Err(error) = local.save(&link.dir) {
                        return link.fail_text(FriendFailure::Other, error.to_string());
                    }
                }
                link.refresh(true);
            }
        });
    }

    /// 名单在服务器上：有自家服务器，而且这台设备登录的是主人（核心名单只归主人）。
    fn uses_server(&self) -> bool {
        self.home.core_url().is_some() && self.server_scope.load(Ordering::SeqCst)
    }

    fn api(&self) -> Result<api::Api, Failure> {
        // 已知隧道不通就立刻失败，不等 30 秒超时。
        if let Some(reason) = self.home.tunnel_outage() {
            return Err(Failure::transport(reason));
        }
        let proxy = self
            .home
            .view()
            .proxy
            .ok_or_else(|| Failure::unauthorized("friends-not-connected"))?;
        let base = self
            .home
            .core_url()
            .ok_or_else(|| Failure::unauthorized("friends-not-connected"))?;
        let client = {
            let mut cached = self.client.lock().expect("friends client lock");
            match cached.as_ref() {
                Some((settings, client)) if *settings == proxy => client.clone(),
                _ => {
                    let client = api::proxied_client(&proxy)?;
                    *cached = Some((proxy.clone(), client.clone()));
                    client
                }
            }
        };
        Ok(api::Api::new(base, client, Vec::new()))
    }

    fn fail(&self, error: wonton_matrix::Error) {
        self.view.send_modify(|view| {
            view.state = FriendsState::Failed {
                problem: Failure::from_matrix(error).problem,
            }
        });
    }

    fn fail_api(&self, failure: Failure) {
        tracing::info!(reason = ?failure.problem.reason, message = %failure.message, "friends api failed");
        self.view.send_modify(|view| {
            view.state = FriendsState::Failed {
                problem: failure.problem,
            }
        });
    }

    fn fail_fixed(&self, reason: FriendFailure) {
        self.view.send_modify(|view| {
            view.state = FriendsState::Failed {
                problem: FriendProblem {
                    reason,
                    http_status: None,
                    matrix_errcode: None,
                    retry_after_ms: None,
                },
            }
        });
    }

    fn fail_text(&self, reason: FriendFailure, message: String) {
        tracing::info!(%message, "friends local store failed");
        self.fail_fixed(reason);
    }

    fn spawn(&self, task: impl Future<Output = ()> + Send + 'static) {
        self.handle.spawn(task);
    }
}

fn name_of(joined: &[(String, String, u64)], user_id: &str) -> Option<String> {
    joined
        .iter()
        .find(|(id, _, _)| id == user_id)
        .map(|(_, name, _)| name.clone())
}

/// 加好友的输入：完整用户 ID，或 matrix.to 的用户链接（含百分号转义，同 [`matrix_target`] 的解析）。
/// 归一成用户 ID；房间链接、普通网址和别的都不是用户，不收。完整校验仍由后面的用户 ID 解析做。
fn friend_user_id(input: &str) -> Option<String> {
    let input = input.trim();
    if let Ok(url) = Url::parse(input) {
        return match matrix_target(&url) {
            Some(MatrixTarget::User { user_id }) => Some(user_id),
            _ => None,
        };
    }
    (input.starts_with('@') && input.contains(':')).then(|| input.to_owned())
}

/// 一次同步里数出来的东西：请求、已加入的好友私聊、屏蔽名单。
struct Sync {
    incoming: Vec<FriendRequest>,
    outgoing: Vec<FriendRequest>,
    joined: Vec<(String, String, u64)>,
}

async fn collect(session: &Arc<wonton_matrix::MatrixSession>) -> Result<Sync, Failure> {
    let mut rows = match session.rooms().await {
        Ok(stream) => Box::pin(stream),
        Err(error) => return Err(Failure::from_matrix(error)),
    };
    // 房间列表第一份快照可能是同步还没送房间时的空表：等到有一间房为止，最多几秒。
    // 直接拿第一份会让好友页在邀请方一侧长时间显示“还没有好友”。
    let rows = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            match rows.next().await {
                Some(rows) if !rows.is_empty() => return rows,
                Some(_) => {}
                None => return Vec::new(),
            }
        }
    })
    .await
    .unwrap_or_default();
    tracing::debug!(rows = rows.len(), "friends sync scan");
    let mut sync = Sync {
        incoming: Vec::new(),
        outgoing: Vec::new(),
        joined: Vec::new(),
    };
    for row in rows.iter().filter(|row| {
        let keep = row.is_direct || row.is_friend;
        tracing::debug!(room = %row.room_id, direct = row.is_direct, friend = row.is_friend, membership = ?row.membership, "friends row {keep}");
        keep
    }) {
        match row.membership {
            wonton_matrix::Membership::Invited => {
                let Ok(Some(invite)) = session.friend_invite(&row.room_id).await else {
                    continue;
                };
                sync.incoming.push(FriendRequest {
                    room_id: invite.room_id.to_string(),
                    user_id: invite.inviter.to_string(),
                    display_name: invite.inviter_name,
                    kind: if invite.friend {
                        FriendRequestKind::Friend
                    } else if invite.bot_contact {
                        FriendRequestKind::BotContact
                    } else {
                        FriendRequestKind::DirectChat
                    },
                    invited_at_ms: invite.invited_at_ms,
                });
            }
            wonton_matrix::Membership::Joined => {
                let Ok(Some(chat)) = session.friend_chat(&row.room_id).await else {
                    tracing::debug!(room = %row.room_id, "friends chat skipped");
                    continue;
                };
                tracing::debug!(room = %row.room_id, peer = ?chat.peer, pending = ?chat.pending_invitee, "friends chat");
                if chat.friend
                    && let Some(peer) = chat.peer
                {
                    sync.joined.push((
                        peer.to_string(),
                        row.name.clone(),
                        chat.invited_at_ms,
                    ));
                }
                if chat.friend
                    && let Some(pending) = chat.pending_invitee
                {
                    sync.outgoing.push(FriendRequest {
                        room_id: chat.room_id.to_string(),
                        user_id: pending.to_string(),
                        display_name: chat.pending_invitee_name,
                        kind: FriendRequestKind::Friend,
                        invited_at_ms: chat.invited_at_ms,
                    });
                }
            }
            _ => {}
        }
    }
    Ok(sync)
}

fn blocked_from(ignored: wonton_matrix::Blocked) -> Vec<FriendBlock> {
    let mut blocked = Vec::new();
    for user in ignored.users {
        blocked.push(FriendBlock {
            target: FriendBlockTarget::User {
                user_id: user.to_string(),
            },
            scope: FriendBlockScope::Invitations,
            blocked_at_ms: 0,
        });
    }
    for server in ignored.servers {
        blocked.push(FriendBlock {
            target: FriendBlockTarget::Server {
                server_name: server.to_string(),
            },
            scope: FriendBlockScope::Invitations,
            blocked_at_ms: 0,
        });
    }
    blocked
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or_default()
}

fn invalid(id: &str) -> CoreError {
    CoreError::InvalidInput {
        reason: text(id, &[]),
    }
}

/// `Url` 在测试里也要用。
pub(crate) fn _base() -> Option<Url> {
    None
}
