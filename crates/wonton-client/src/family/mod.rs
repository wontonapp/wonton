//! 成员页（ADR-0088）：设置里的“成员”。主人和管理员在这里领人进门、批加入请求、移除丢了的手机，
//! 主人还在这里给人换角色。角色页也在这一块：邀请时选的就是它。
//!
//! 状态分三份：成员列表、加入请求和角色，各自的 `loading`/`problem` 分开，一次失败不糊到别处。
//! 成员和请求只有“管理成员”能读，角色只有主人能读；读不到的接口按空列表处理，页面按
//! [`FamilyAccess`](wonton_protocol::FamilyAccess) 决定显示哪些操作。
//!
//! 页面开着时每几秒重取一次成员和请求：对方扫码、管理员在别的设备上批了，这里都自己跟上，不用退出重进。

mod api;
mod roles;

use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use tokio::runtime::Handle;
use tokio::sync::watch;
use tokio::sync::watch::Sender as WatchSender;
use wonton_protocol::{
    CoreError, FamilyAccess, FamilyAction, FamilyFailure, FamilyProblem, FamilyViewModel,
    HomeStatus, JoinRequestRow, Member, MemberCode, MemberInvite, MemberInviteView, MemberList,
    MemberPairingCode, MemberRemoval, MemberRow, RoleDef,
};

use crate::home::HomeLink;
use crate::plugins::PluginsLink;
use crate::plugins::api::{CoreApi, READ_TIMEOUT};
use crate::scope::reuse;

use api::REMOVE_TIMEOUT;

/// 页面开着时多久重取一次：核心那边的请求、别人的批准都靠它跟上。
const POLL_INTERVAL: Duration = Duration::from_secs(5);

pub(crate) struct FamilyLink {
    runtime: Handle,
    home: Arc<HomeLink>,
    plugins: Arc<PluginsLink>,
    view: WatchSender<FamilyViewModel>,
    /// 页面开着：定时重取。
    live: AtomicBool,
    /// 有跟随时钟在跑；停下时清掉，回到页面再起。
    following: AtomicBool,
    /// 同一时刻只跑一次取数。
    queue: tokio::sync::Mutex<()>,
    /// 每次重取加一，只有最后发起的那次能写进 ViewModel。
    epoch: AtomicU64,
}

impl FamilyLink {
    pub(crate) fn new(
        runtime: Handle,
        home: Arc<HomeLink>,
        plugins: Arc<PluginsLink>,
        view: Option<WatchSender<FamilyViewModel>>,
    ) -> Arc<Self> {
        let link = Arc::new(Self {
            runtime,
            home,
            plugins,
            view: reuse(view, FamilyViewModel::default()),
            live: AtomicBool::new(false),
            following: AtomicBool::new(false),
            queue: tokio::sync::Mutex::default(),
            epoch: AtomicU64::new(0),
        });
        link.ensure_following();
        link
    }

    pub(crate) fn sender(&self) -> WatchSender<FamilyViewModel> {
        self.view.clone()
    }

    pub(crate) fn subscribe(&self) -> watch::Receiver<FamilyViewModel> {
        self.view.subscribe()
    }

    fn spawn(&self, future: impl Future<Output = ()> + Send + 'static) {
        self.runtime.spawn(future);
    }

    /// 没有跟随时钟就起一个；已经有在跑的就不重复起。
    fn ensure_following(self: &Arc<Self>) {
        if self.following.swap(true, Ordering::SeqCst) {
            return;
        }
        let weak = Arc::downgrade(self);
        self.runtime.spawn(async move { follow_home(weak).await });
    }

    fn api(&self) -> Result<CoreApi, FamilyProblem> {
        self.plugins.api().map_err(|_| api::not_connected())
    }

    fn available(&self) -> bool {
        !self.plugins.is_external()
            && self.home.view().status == HomeStatus::Connected
            && self.home.tunnel_outage().is_none()
    }

    pub(crate) fn dispatch(self: &Arc<Self>, action: FamilyAction) -> Result<(), CoreError> {
        match action {
            FamilyAction::RefreshFamily => {
                self.live.store(true, Ordering::SeqCst);
                self.ensure_following();
                self.refresh(true);
            }
            FamilyAction::LeaveFamily => self.live.store(false, Ordering::SeqCst),
            FamilyAction::RefreshRoles => self.refresh_roles(),
            FamilyAction::InviteMember {
                display_name,
                username,
                private_bot,
                role_id,
            } => self.invite(display_name, username, private_bot, role_id),
            FamilyAction::ResendMemberCode { username } => self.code(username, false),
            FamilyAction::RevokeMemberCode { username } => self.code(username, true),
            FamilyAction::RemoveMember {
                username,
                delete_home,
                keep_bot_data,
            } => self.remove(username, delete_home, keep_bot_data),
            FamilyAction::DismissMemberInvite => {
                self.view.send_modify(|view| view.invite = None);
            }
            FamilyAction::RefreshJoinRequests => self.refresh_requests(true),
            FamilyAction::ApproveJoinRequest { id } => self.decide_request(id, true),
            FamilyAction::RejectJoinRequest { id } => self.decide_request(id, false),
            action => {
                if let Some(request) = self.roles_action(action) {
                    self.mutate(request);
                }
            }
        }
        Ok(())
    }

    /// 取成员、角色、加入请求和自己的资料。`loading` 只在打开页面和手动刷新时翻开，轮询不翻。
    fn refresh(self: &Arc<Self>, loading: bool) {
        let link = Arc::clone(self);
        self.spawn(async move {
            link.fetch(loading).await;
        });
    }

    async fn fetch(self: Arc<Self>, loading: bool) {
        let epoch = self.epoch.fetch_add(1, Ordering::SeqCst) + 1;
        let _guard = self.queue.lock().await;
        if loading {
            self.view.send_modify(|view| {
                view.members.loading = true;
                view.join_requests.loading = true;
            });
        }
        let access = self.access().await;
        let roles = self.fetch_roles().await;
        let members = self.members().await;
        let requests = self.requests().await;
        if self.epoch.load(Ordering::SeqCst) != epoch {
            return;
        }
        self.view.send_modify(|view| {
            view.members.loading = false;
            view.join_requests.loading = false;
            let access_problem = access.as_ref().err().cloned();
            match access {
                Ok(access) => {
                    view.access = access;
                    apply_roles(&mut view.members.members, &view.roles.roles);
                    apply_role_names(&mut view.join_requests.requests, &view.roles.roles);
                }
                // 自己的资料取不到更根本：成员和请求的结果不盖掉它。
                Err(problem) => view.members.problem = Some(problem),
            }
            view.roles.loading = false;
            match roles {
                Ok(list) => {
                    view.roles.roles = list.roles;
                    view.roles.default_invite_role = Some(list.default_invite_role);
                    view.roles.problem = None;
                    apply_roles(&mut view.members.members, &view.roles.roles);
                    apply_role_names(&mut view.join_requests.requests, &view.roles.roles);
                }
                Err(problem) if problem.reason == FamilyFailure::PermissionRequired => {
                    view.roles.problem = None;
                }
                Err(problem) => view.roles.problem = Some(problem),
            }
            match members {
                Ok(list) => {
                    view.members.members = member_items(list, &view.roles.roles);
                    if access_problem.is_none() {
                        view.members.problem = None;
                    }
                }
                // 取失败保留上次的列表，等恢复再换。
                Err(problem) => {
                    if access_problem.is_none() {
                        view.members.problem = Some(problem);
                    }
                }
            }
            match requests {
                Ok(list) => {
                    view.join_requests.requests = request_items(list, &view.roles.roles);
                    view.join_requests.problem = None;
                }
                Err(problem) => view.join_requests.problem = Some(problem),
            }
        });
    }

    async fn access(&self) -> Result<FamilyAccess, FamilyProblem> {
        let api = self.api()?;
        let me: wonton_protocol::Me = api
            .call(reqwest::Method::GET, &["me"], None::<&()>, READ_TIMEOUT)
            .await?;
        Ok(FamilyAccess {
            user_id: me.user_id,
            username: me.username,
            display_name: me.display_name,
            is_owner: me.is_owner,
            role_id: me.role_id,
            role_name: me.role_name,
            permissions: me.permissions,
        })
    }

    async fn members(&self) -> Result<MemberList, FamilyProblem> {
        let api = self.api()?;
        api.call(
            reqwest::Method::GET,
            &["members"],
            None::<&()>,
            READ_TIMEOUT,
        )
        .await
    }

    async fn requests(&self) -> Result<wonton_protocol::JoinRequestList, FamilyProblem> {
        let api = self.api()?;
        api.call(
            reqwest::Method::GET,
            &["members", "join-requests"],
            None::<&()>,
            READ_TIMEOUT,
        )
        .await
    }

    fn refresh_requests(self: &Arc<Self>, loading: bool) {
        let link = Arc::clone(self);
        self.spawn(async move {
            if loading {
                link.view
                    .send_modify(|view| view.join_requests.loading = true);
            }
            let fetched = link.requests().await;
            link.view.send_modify(|view| {
                view.join_requests.loading = false;
                match fetched {
                    Ok(list) => {
                        view.join_requests.requests = request_items(list, &view.roles.roles);
                        view.join_requests.problem = None;
                    }
                    Err(problem) => view.join_requests.problem = Some(problem),
                }
            });
        });
    }

    fn invite(
        self: &Arc<Self>,
        display_name: String,
        username: Option<String>,
        private_bot: bool,
        role_id: Option<String>,
    ) {
        let link = Arc::clone(self);
        self.spawn(async move {
            let chooses_role = link.view.borrow().access.chooses_role();
            let role_id = chooses_role.then_some(role_id).flatten();
            let invite = MemberInvite {
                display_name,
                username,
                private_bot,
                role_id,
            };
            let done = async {
                let api = link.api()?;
                let member: Member = api
                    .call(
                        reqwest::Method::POST,
                        &["members"],
                        Some(&invite),
                        READ_TIMEOUT,
                    )
                    .await?;
                let code: MemberPairingCode = api
                    .call(
                        reqwest::Method::POST,
                        &["members", &member.username, "pairing-code"],
                        None::<&()>,
                        READ_TIMEOUT,
                    )
                    .await?;
                Ok::<_, FamilyProblem>((member, code))
            }
            .await;
            match done {
                Ok((member, code)) => {
                    link.view.send_modify(|view| {
                        view.invite = Some(MemberInviteView {
                            username: member.username,
                            display_name: member.display_name,
                            code: code.code,
                            expires_at_ms: code.expires_at_ms,
                        });
                    });
                    link.refresh(false);
                }
                Err(problem) => link
                    .view
                    .send_modify(|view| view.members.problem = Some(problem)),
            }
        });
    }

    /// 重发（`revoke` 为假）或作废（为真）这个人的码。
    fn code(self: &Arc<Self>, username: String, revoke: bool) {
        let link = Arc::clone(self);
        self.spawn(async move {
            let done = async {
                let api = link.api()?;
                if revoke {
                    api.call_empty(
                        reqwest::Method::DELETE,
                        &["members", &username, "pairing-code"],
                        None::<&()>,
                        READ_TIMEOUT,
                    )
                    .await?;
                    Ok::<_, FamilyProblem>(None)
                } else {
                    let code: MemberPairingCode = api
                        .call(
                            reqwest::Method::POST,
                            &["members", &username, "pairing-code"],
                            None::<&()>,
                            READ_TIMEOUT,
                        )
                        .await?;
                    Ok(Some(code))
                }
            }
            .await;
            link.view.send_modify(|view| match done {
                Ok(code) => {
                    view.members.problem = None;
                    match code {
                        Some(code) => {
                            let display_name = view
                                .members
                                .members
                                .iter()
                                .find(|member| member.username == username)
                                .map_or_else(
                                    || username.clone(),
                                    |member| member.display_name.clone(),
                                );
                            view.invite = Some(MemberInviteView {
                                username: username.clone(),
                                display_name,
                                code: code.code,
                                expires_at_ms: code.expires_at_ms,
                            });
                        }
                        None => view.invite = None,
                    }
                }
                Err(problem) => view.members.problem = Some(problem),
            });
            link.refresh(false);
        });
    }

    fn remove(self: &Arc<Self>, username: String, delete_home: bool, keep_bot_data: bool) {
        let link = Arc::clone(self);
        self.spawn(async move {
            let removal = MemberRemoval {
                delete_home,
                keep_bot_data,
            };
            let done = async {
                let api = link.api()?;
                api.call::<serde_json::Value>(
                    reqwest::Method::POST,
                    &["members", &username, "remove"],
                    Some(&removal),
                    REMOVE_TIMEOUT,
                )
                .await
                .map(drop)
            }
            .await;
            link.view.send_modify(|view| match done {
                Ok(()) => {
                    view.members.problem = None;
                    if view
                        .invite
                        .as_ref()
                        .is_some_and(|code| code.username == username)
                    {
                        view.invite = None;
                    }
                }
                Err(problem) => view.members.problem = Some(problem),
            });
            link.refresh(false);
        });
    }

    fn decide_request(self: &Arc<Self>, id: String, approve: bool) {
        let link = Arc::clone(self);
        self.spawn(async move {
            let done = async {
                let api = link.api()?;
                let decision = if approve { "approve" } else { "reject" };
                let segments = ["members", "join-requests", id.as_str(), decision];
                api.call_empty(reqwest::Method::POST, &segments, None::<&()>, READ_TIMEOUT)
                    .await
                    .map(drop)
            }
            .await;
            link.view.send_modify(|view| match done {
                Ok(()) => view.join_requests.problem = None,
                Err(problem) => view.join_requests.problem = Some(problem),
            });
            link.refresh(false);
        });
    }
}

/// 成员记录配上角色名和模板；码过期按本机时间算。
fn member_items(list: MemberList, roles: &[RoleDef]) -> Vec<MemberRow> {
    let now = now_ms();
    list.members
        .into_iter()
        .filter(|member| member.status != wonton_protocol::MemberStatus::Removed)
        .map(|member| {
            let role = roles.iter().find(|role| role.id == member.role_id);
            MemberRow {
                code: member
                    .pairing_code_expires_at_ms
                    .map(|expires_at_ms| MemberCode {
                        expires_at_ms,
                        expired: expires_at_ms <= now,
                    }),
                role_name: role.map(|role| role.name.clone()),
                role_template: role.and_then(|role| role.template),
                username: member.username,
                user_id: member.user_id,
                display_name: member.display_name,
                email: member.email,
                status: member.status,
                role_id: member.role_id,
                private_bot: member.private_bot,
            }
        })
        .collect()
}

fn request_items(list: wonton_protocol::JoinRequestList, roles: &[RoleDef]) -> Vec<JoinRequestRow> {
    let mut rows: Vec<JoinRequestRow> = list
        .requests
        .into_iter()
        .map(|request| {
            let role = roles.iter().find(|role| role.id == request.role_id);
            JoinRequestRow {
                role_name: role.map(|role| role.name.clone()),
                role_template: role.and_then(|role| role.template),
                id: request.id,
                username: request.username,
                display_name: request.display_name,
                role_id: request.role_id,
                device_name: request.device_name,
                requested_at_ms: request.requested_at_ms,
            }
        })
        .collect();
    rows.sort_by_key(|row| std::cmp::Reverse(row.requested_at_ms));
    rows
}

fn apply_roles(rows: &mut [MemberRow], roles: &[RoleDef]) {
    for row in rows {
        let role = roles.iter().find(|role| role.id == row.role_id);
        row.role_name = role.map(|role| role.name.clone());
        row.role_template = role.and_then(|role| role.template);
    }
}

fn apply_role_names(rows: &mut [JoinRequestRow], roles: &[RoleDef]) {
    for row in rows {
        let role = roles.iter().find(|role| role.id == row.role_id);
        row.role_name = role.map(|role| role.name.clone());
        row.role_template = role.and_then(|role| role.template);
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as u64)
}

/// 连上家里服务器时取一次；掉线后页面开着就继续按间隔重试。
async fn follow_home(link: std::sync::Weak<FamilyLink>) {
    loop {
        let Some(link) = link.upgrade() else { return };
        if link.available() {
            link.refresh(false);
            break;
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    loop {
        let Some(arc) = link.upgrade() else { return };
        if !arc.live.load(Ordering::SeqCst) {
            arc.following.store(false, Ordering::SeqCst);
            // 停下的一刻可能又有人进了页面：接着跟，别把新请求丢了。
            if arc.live.load(Ordering::SeqCst) {
                if arc
                    .following
                    .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                    .is_err()
                {
                    return;
                }
                continue;
            }
            return;
        }
        tokio::time::sleep(POLL_INTERVAL).await;
        let Some(arc) = link.upgrade() else { return };
        if arc.live.load(Ordering::SeqCst) && arc.available() {
            arc.refresh(false);
        }
    }
}
