//! 聊天：经隧道登录主人的 Matrix 账号，房间列表、时间线、收发消息和端到端加密（ADR-0003、ADR-0005，以及
//! ADR-0009 的“接口：App 登录和使用 Matrix”）。
//!
//! 跟着家里的连接走：本地代理就绪（或者换了）就登录。上次登录的是同一台服务器就用存下来的登录状态恢复，不经服务器；
//! 否则经隧道向核心要一次性的登录令牌，用它登录 `matrix.<基础域名>`。所有 Matrix 请求都走本地代理，Tuwunel 只被当成
//! 标准的 Matrix 服务器（ADR-0015）。

mod ask;
mod confirmation;
mod contacts;
mod display_name;
mod login;
mod onboarding;
mod pending;
mod read_mark;
mod recovery;
mod rich;
mod session;
mod sign_in;
mod storage;
mod timeline;
mod view;

use std::future::Future;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, MutexGuard, Weak};

use futures_util::StreamExt;
use tokio::runtime::Handle;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use wonton_matrix::{ChatRoom, Error, MatrixSession, OwnedRoomId, OwnedUserId, SyncStatus};
use wonton_protocol::{
    Action, Answer, ChatAccount, ChatFailure, ChatProblem, ChatSync, ChatViewModel,
    ConfirmationAnswer, CoreError, DeviceVerification, EncryptionStatus, HomeStatus, HomeViewModel,
    ImageUpload, MediaHandle, ProxySettings, PushStatus, RecoveryStatus,
};

pub(crate) use confirmation::{PushedCard, pushed_card};
pub(crate) use login::proxy_url;
pub use timeline::TimelineSubscription;
pub(crate) use view::{failure, summary};

use crate::home::HomeLink;
use confirmation::{Answers, Cards, core_account};
use storage::ChatStorage;
use view::{core_error, room_row, sort_rooms};

/// 向前翻页一次最多加载的事件数。
const PAGE: u16 = 30;

pub(crate) struct ChatLink {
    runtime: Handle,
    home: Arc<HomeLink>,
    storage: ChatStorage,
    view: watch::Sender<ChatViewModel>,
    /// 当前的会话和它的后台任务；换代理或重新登录时整个替换。锁保证同一时刻只有一次登录。
    active: tokio::sync::Mutex<Option<Active>>,
    /// 当前会话，给不阻塞的 `dispatch` 用。
    session: Mutex<Option<Arc<MatrixSession>>>,
    /// 订阅着时间线的房间。同一时间只打开一个房间。
    open_room: Mutex<Option<(OwnedRoomId, Arc<ChatRoom>)>>,
    /// 这台设备回答确认卡片的进展，时间线的订阅跟着它更新卡片。
    answers: watch::Sender<Answers>,
    /// 本会话里亲眼看到从没结束变成成功的插件 ID：插件列表多半还是旧的，补取的逻辑在插件那头。
    card_success: mpsc::UnboundedSender<String>,
    /// 正在重置恢复。途中 SDK 会报“未设置”，不能覆盖“处理中”或触发自动开启。
    resetting_recovery: AtomicBool,
    /// 主人的默认 Agent 在 Matrix 上的用户 ID，聊天列表里它的房间排第一。
    primary_bot: Arc<Mutex<Option<String>>>,
    /// 房间列表的基准顺序，显示时把默认 Agent 的房间提到最前。
    room_order: Arc<onboarding::RoomOrder>,
    /// 发过消息的房间，引导提示用。
    sent: watch::Sender<Vec<String>>,
    /// 还没登录 Matrix 时改的显示名，登录后再设。
    display_name: Mutex<Option<String>>,
}

struct Active {
    tasks: Vec<JoinHandle<()>>,
}

impl Drop for Active {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

impl ChatLink {
    pub(crate) fn new(
        runtime: Handle,
        home: Arc<HomeLink>,
        data_dir: &Path,
        card_success: mpsc::UnboundedSender<String>,
    ) -> Result<Arc<Self>, CoreError> {
        let storage = ChatStorage::open(data_dir)?;
        let mut encryption = EncryptionStatus {
            verification: DeviceVerification::Unknown,
            recovery: RecoveryStatus::Unknown,
            recovery_key: storage.recovery_key()?,
            backup_reminder: None,
        };
        recovery::update_reminder(&mut encryption);
        let view = watch::Sender::new(ChatViewModel {
            account: ChatAccount::SignedOut,
            sync: ChatSync::Idle,
            encryption,
            rooms: Vec::new(),
            problem: None,
            push: PushStatus::NoEndpoint,
            pending_confirmations: Vec::new(),
            settled_confirmations: Vec::new(),
            bots: Vec::new(),
        });
        let chat = Arc::new(Self {
            runtime: runtime.clone(),
            home: home.clone(),
            storage,
            view,
            active: tokio::sync::Mutex::new(None),
            session: Mutex::new(None),
            open_room: Mutex::new(None),
            answers: watch::Sender::default(),
            card_success,
            resetting_recovery: AtomicBool::default(),
            primary_bot: Arc::new(Mutex::new(None)),
            room_order: Arc::new(onboarding::RoomOrder::new()),
            sent: watch::Sender::new(Vec::new()),
            display_name: Mutex::new(None),
        });
        runtime.spawn(follow_home(Arc::downgrade(&chat), home.subscribe()));
        Ok(chat)
    }

    pub(crate) fn view(&self) -> ChatViewModel {
        self.view.borrow().clone()
    }

    pub(crate) fn subscribe(&self) -> watch::Receiver<ChatViewModel> {
        self.view.subscribe()
    }

    pub(crate) fn subscribe_sent(&self) -> watch::Receiver<Vec<String>> {
        self.sent.subscribe()
    }

    pub(crate) fn set_primary_bot(&self, bot: Option<String>) {
        let changed = {
            let mut current = lock(&self.primary_bot);
            if *current == bot {
                false
            } else {
                *current = bot.clone();
                true
            }
        };
        if changed {
            let rows = self.room_order.display(bot.as_deref());
            self.view.send_modify(|view| view.rooms = rows);
        }
    }

    /// App 回到前台或家里的连接恢复：登录失败（不是被吊销）的话重新登录；已经登录的接着用同一个会话，同步和发送队列
    /// 马上恢复。换会话的话，打开着的房间还订阅着旧会话，再也收不到更新。
    pub(crate) fn foregrounded(self: &Arc<Self>) {
        let view = self.view();
        // 还没连上家里、或者正在登录（家里的连接一就绪就会登录）时不插手；被吊销只能重新配对。Android 启动时
        // 也会发一次“回到前台”，正赶上第一次登录，这时再登录一遍会在同一个存储上开两个会话。
        match &view.account {
            ChatAccount::Failed { reason, .. } if *reason != ChatFailure::Unauthorized => {
                if let Some(proxy) = self.home.view().proxy {
                    self.runtime.spawn(self.clone().connect(proxy));
                }
            }
            ChatAccount::SignedIn { .. } => {
                if let Some(session) = lock(&self.session).clone() {
                    session.resume();
                }
            }
            ChatAccount::Failed { .. } | ChatAccount::SignedOut | ChatAccount::SigningIn => {}
        }
    }

    /// 处理聊天操作。参数不合法、还没登录时直接返回错误；网络操作在后台执行，失败放进 `problem`。
    pub(crate) fn dispatch(self: &Arc<Self>, action: Action) -> Result<(), CoreError> {
        match action {
            Action::DismissChatProblem => {
                self.view.send_modify(|view| view.problem = None);
                Ok(())
            }
            Action::JoinRoom { room_id } => {
                let (session, room_id) = self.target(&room_id)?;
                self.spawn_op("加入房间", async move { session.join_room(&room_id).await });
                Ok(())
            }
            Action::LeaveRoom { room_id } => {
                let (session, room_id) = self.target(&room_id)?;
                self.spawn_op(
                    "离开房间",
                    async move { session.leave_room(&room_id).await },
                );
                Ok(())
            }
            Action::StartDirectChat { user_id } => self.start_direct_chat(&user_id),
            Action::SendText { room_id, body } => self.send_text(&room_id, &body),
            Action::SendImage { room_id, image } => self.send_image(&room_id, image),
            Action::RetrySend { room_id, item_id } => {
                let room = self.opened(&room_id)?;
                let sent = self.sent.clone();
                self.spawn_op_with_success(
                    "重发消息",
                    async move { room.retry_send(&item_id).await },
                    move || onboarding::record_sent(&sent, room_id),
                );
                Ok(())
            }
            Action::CancelSend { room_id, item_id } => {
                let room = self.opened(&room_id)?;
                self.spawn_op("取消发送", async move { room.cancel_send(&item_id).await });
                Ok(())
            }
            Action::PaginateBackwards { room_id } => {
                let room = self.opened(&room_id)?;
                self.spawn_op("加载更早的消息", async move {
                    room.paginate_backwards(PAGE).await.map(drop)
                });
                Ok(())
            }
            Action::MarkRoomRead { room_id } => {
                let chat = self.clone();
                let (_, room_id) = self.target(&room_id)?;
                self.spawn_op("标为已读", async move {
                    chat.room(&room_id).await?.mark_as_read().await.map(drop)
                });
                Ok(())
            }
            Action::ResetRecovery => self.reset_recovery(),
            Action::ConfirmRecoveryKeySaved => self.confirm_recovery_key_saved(),
            Action::RecoverEncryption { recovery_key } => self.recover(&recovery_key),
            Action::AnswerConfirmation {
                id,
                answer,
                choice_id,
            } => self.answer_confirmation(id, answer, choice_id),
            Action::StopConfirmationWindow { id } => self.stop_confirmation_window(id),
            _ => Err(invalid("不是聊天操作")),
        }
    }

    fn send_text(self: &Arc<Self>, room_id: &str, body: &str) -> Result<(), CoreError> {
        let body = body.trim().to_owned();
        if body.is_empty() {
            return Err(invalid("消息不能为空"));
        }
        let chat = self.clone();
        let sent = self.sent.clone();
        let (_, room_id) = self.send_target(room_id)?;
        let sent_room = room_id.to_string();
        self.spawn_op_with_success(
            "发送消息",
            async move { chat.room(&room_id).await?.send_text(&body).await },
            move || onboarding::record_sent(&sent, sent_room),
        );
        Ok(())
    }

    fn confirm_recovery_key_saved(&self) -> Result<(), CoreError> {
        self.storage.forget_recovery_key()?;
        self.view.send_modify(|view| {
            view.encryption.recovery_key = None;
            recovery::update_reminder(&mut view.encryption);
        });
        Ok(())
    }

    /// 订阅一个已加入房间的时间线。打开另一个房间会关掉前一个的订阅。
    pub(crate) async fn open_timeline(
        self: &Arc<Self>,
        room_id: &str,
    ) -> Result<Arc<TimelineSubscription>, CoreError> {
        let (session, room_id) = self.target(room_id)?;
        let room = Arc::new(
            session
                .open_room(&room_id)
                .await
                .map_err(|e| core_error(&e))?,
        );
        *lock(&self.open_room) = Some((room_id, room.clone()));
        let cards = Cards::new(session.user_id(), self.answers.subscribe());
        Ok(TimelineSubscription::start(&self.runtime, room, cards))
    }

    /// 凭句柄下载媒体原文件（加密附件在本机解密）。
    pub(crate) async fn load_media(&self, media: MediaHandle) -> Result<Vec<u8>, CoreError> {
        let session = self.current()?;
        session
            .download_media(&media.token)
            .await
            .map_err(|e| core_error(&e))
    }

    /// 发一条文字，不打开房间：传完文件发链接时，主人可能已经在看别的房间了，打开这个房间会打断那边的时间线。
    pub(crate) async fn send_text_unopened(
        &self,
        room_id: &str,
        body: &str,
    ) -> Result<(), CoreError> {
        let (session, room_id) = self.target(room_id)?;
        session
            .send_text_to(&room_id, body)
            .await
            .map_err(|e| core_error(&e))?;
        onboarding::record_sent(&self.sent, room_id.to_string());
        Ok(())
    }

    fn start_direct_chat(self: &Arc<Self>, user_id: &str) -> Result<(), CoreError> {
        let user: OwnedUserId = user_id
            .parse()
            .map_err(|_| invalid("ID 的格式不对，应该像 @名字:服务器"))?;
        let session = self.current()?;
        if session.direct_chat_with(&user).is_some() {
            return Ok(());
        }
        // 和 Bot 的主房间一样端到端加密：推送只带房间和事件 ID，正文在手机上解密（ADR-0003 的“设计：加密推送”）。
        self.spawn_op("发起私聊", async move {
            session.create_direct_chat(&user, true).await.map(drop)
        });
        Ok(())
    }

    fn send_image(self: &Arc<Self>, room_id: &str, image: ImageUpload) -> Result<(), CoreError> {
        if !image.mime_type.starts_with("image/") {
            return Err(invalid(&format!("不是图片类型：{}", image.mime_type)));
        }
        let chat = self.clone();
        let sent = self.sent.clone();
        let (_, room_id) = self.send_target(room_id)?;
        let sent_room = room_id.to_string();
        let upload = wonton_matrix::ImageUpload {
            filename: image.filename,
            mime_type: image.mime_type,
            data: image.data,
            width: image.width,
            height: image.height,
            thumbnail: image
                .thumbnail
                .map(|thumbnail| wonton_matrix::ImageThumbnail {
                    mime_type: thumbnail.mime_type,
                    data: thumbnail.data,
                    width: thumbnail.width,
                    height: thumbnail.height,
                }),
        };
        self.spawn_op_with_success(
            "发送图片",
            async move { chat.room(&room_id).await?.send_image(upload).await },
            move || onboarding::record_sent(&sent, sent_room),
        );
        Ok(())
    }

    /// 主人点了确认卡片上的按钮：经隧道发给核心，进展放进 `answers`。同一张卡片在发或者答完了就不再发：核心那边
    /// 也只收一次，这里是不让连点变成“回答过了”的报错。
    fn answer_confirmation(
        &self,
        id: String,
        answer: Answer,
        choice_id: Option<String>,
    ) -> Result<(), CoreError> {
        if answer == Answer::Unknown {
            return Err(invalid("这个 App 不认识这个回答"));
        }
        let not_connected = || CoreError::NotReady {
            reason: "还没连上家里的服务器".to_owned(),
        };
        let proxy = self.home.view().proxy.ok_or_else(not_connected)?;
        let core_url = self.home.core_url().ok_or_else(not_connected)?;
        if !self
            .answers
            .send_if_modified(|answers| answers.start(&id, answer, choice_id.clone()))
        {
            return Ok(());
        }
        let answers = self.answers.clone();
        self.runtime.spawn(async move {
            let body = ConfirmationAnswer { answer, choice_id };
            let outcome = confirmation::send_answer(&core_url, &proxy, &id, body).await;
            answers.send_modify(|answers| answers.finish(id, outcome));
        });
        Ok(())
    }

    /// 主人点了卡片上放行时段的“停止”：经隧道发给核心，进展放进 `answers`。
    fn stop_confirmation_window(&self, id: String) -> Result<(), CoreError> {
        let not_connected = || CoreError::NotReady {
            reason: "还没连上家里的服务器".to_owned(),
        };
        let proxy = self.home.view().proxy.ok_or_else(not_connected)?;
        let core_url = self.home.core_url().ok_or_else(not_connected)?;
        if !self
            .answers
            .send_if_modified(|answers| answers.start_stop(&id))
        {
            return Ok(());
        }
        let answers = self.answers.clone();
        self.runtime.spawn(async move {
            let outcome = confirmation::send_stop(&core_url, &proxy, &id).await;
            answers.send_modify(|answers| answers.finish_stop(id, outcome));
        });
        Ok(())
    }

    /// 在后台执行一个操作，成功时先回调，失败时把原因放进 `problem`。
    fn spawn_op_with_success(
        &self,
        what: &'static str,
        op: impl Future<Output = Result<(), Error>> + Send + 'static,
        success: impl FnOnce() + Send + 'static,
    ) {
        let view = self.view.clone();
        self.runtime.spawn(async move {
            match op.await {
                Ok(()) => success(),
                Err(error) => {
                    tracing::info!("{what}失败：{error}");
                    let reason = failure(&error);
                    let message = if reason == ChatFailure::TransportUnavailable {
                        view::NOT_CONNECTED.to_owned()
                    } else {
                        format!("{what}失败：{error}")
                    };
                    view.send_modify(|view| view.problem = Some(ChatProblem { reason, message }));
                }
            }
        });
    }

    /// 在后台执行一个操作，失败时把原因放进 `problem`。
    fn spawn_op(
        &self,
        what: &'static str,
        op: impl Future<Output = Result<(), Error>> + Send + 'static,
    ) {
        self.spawn_op_with_success(what, op, || {});
    }

    /// 推送的状态由 `push` 模块维护，放在聊天的 ViewModel 里。
    pub(crate) fn set_push(&self, status: PushStatus) {
        self.view.send_modify(|view| view.push = status);
    }

    pub(crate) fn current(&self) -> Result<Arc<MatrixSession>, CoreError> {
        lock(&self.session)
            .clone()
            .ok_or_else(|| CoreError::NotReady {
                reason: "还没有登录 Matrix".to_owned(),
            })
    }

    fn target(&self, room_id: &str) -> Result<(Arc<MatrixSession>, OwnedRoomId), CoreError> {
        let room_id = room_id
            .parse()
            .map_err(|e| invalid(&format!("房间 ID 不合法：{e}")))?;
        Ok((self.current()?, room_id))
    }

    /// 要往房间里发东西时用它，只读的房间拒绝。
    fn send_target(&self, room_id: &str) -> Result<(Arc<MatrixSession>, OwnedRoomId), CoreError> {
        let read_only = self
            .view
            .borrow()
            .rooms
            .iter()
            .any(|row| row.room_id == room_id && row.is_read_only);
        if read_only {
            return Err(invalid("这个房间只放通知，不能发消息"));
        }
        self.target(room_id)
    }

    fn opened(&self, room_id: &str) -> Result<Arc<ChatRoom>, CoreError> {
        match &*lock(&self.open_room) {
            Some((id, room)) if id.as_str() == room_id => Ok(room.clone()),
            _ => Err(CoreError::NotReady {
                reason: format!("要先订阅房间 {room_id} 的时间线"),
            }),
        }
    }

    /// 已打开的房间直接用，否则打开它。
    ///
    /// 打开会把 Sliding Sync 的房间订阅换成这个房间（同一时间只订阅一个，见 `MatrixSession::open_room`），正在看的
    /// 房间就只剩房间列表带来的最新一条，时间线会断。现在界面只在打开着的房间里发消息、标为已读，走不到这一步；
    /// 以后在列表上做“标为已读”这类不进房间的操作时，要改成不动订阅的写法（比如直接对 SDK 的 `Room` 发回执）。
    async fn room(&self, room_id: &OwnedRoomId) -> Result<Arc<ChatRoom>, Error> {
        if let Some((id, room)) = &*lock(&self.open_room)
            && id == room_id
        {
            return Ok(room.clone());
        }
        let session = self.current().map_err(|e| Error::NotReady(e.to_string()))?;
        Ok(Arc::new(session.open_room(room_id).await?))
    }
}

/// 跟着家里的连接：本地代理就绪或换了就（重新）登录；隧道恢复连接时，登录失败过就再试一次。
async fn follow_home(chat: Weak<ChatLink>, mut home: watch::Receiver<HomeViewModel>) {
    let mut proxy: Option<ProxySettings> = None;
    let mut connected = false;
    loop {
        let (next_proxy, now_connected) = {
            let view = home.borrow_and_update();
            (view.proxy.clone(), view.status == HomeStatus::Connected)
        };
        let Some(link) = chat.upgrade() else { return };
        let reconnected = now_connected && !connected;
        connected = now_connected;
        if next_proxy != proxy {
            proxy.clone_from(&next_proxy);
            match next_proxy {
                Some(next) => {
                    link.runtime.spawn(link.clone().connect(next));
                }
                None => link.disconnect().await,
            }
        } else if reconnected {
            link.foregrounded();
        }
        drop(link);
        if home.changed().await.is_err() {
            return;
        }
    }
}

impl ChatLink {
    /// 用 `proxy` 登录或恢复会话，然后开始同步。
    async fn connect(self: Arc<Self>, proxy: ProxySettings) {
        let mut active = self.active.lock().await;
        // 等锁时可能又换了连接：这个代理已经没了，不能凭它动下一台服务器的登录状态。
        let home = self.home.view();
        let Some(server_id) = home
            .server_id
            .filter(|_| home.proxy.as_ref() == Some(&proxy))
        else {
            return;
        };
        if let Some(old) = active.take() {
            self.forget_current().await;
            drop(old);
        }
        self.view
            .send_modify(|view| view.account = ChatAccount::SigningIn);
        let session = match self.sign_in(&proxy, server_id).await {
            Ok(session) => Arc::new(session),
            Err((reason, message)) => {
                tracing::info!("登录 Matrix 失败：{message}");
                self.view.send_modify(|view| {
                    view.account = ChatAccount::Failed { reason, message };
                    view.sync = ChatSync::Idle;
                });
                return;
            }
        };
        *lock(&self.session) = Some(session.clone());
        self.apply_pending_display_name(&session);
        self.view.send_modify(|view| {
            view.account = ChatAccount::SignedIn {
                user_id: session.user_id().to_string(),
                device_id: session.device_id().to_string(),
            };
            view.sync = ChatSync::Starting;
        });
        let tasks = self.observe(&session);
        *active = Some(Active { tasks });
        drop(active);
        if let Err(error) = session.start_sync().await {
            self.view.send_modify(|view| {
                view.sync = ChatSync::Failed {
                    reason: failure(&error),
                    message: error.to_string(),
                };
            });
        }
    }

    /// 家里的连接没了：停掉同步，留着登录状态，下次连上时恢复。
    async fn disconnect(&self) {
        let mut active = self.active.lock().await;
        if active.take().is_some() {
            self.forget_current().await;
        }
        self.view.send_modify(|view| {
            view.account = ChatAccount::SignedOut;
            view.sync = ChatSync::Idle;
        });
    }

    async fn forget_current(&self) {
        *lock(&self.open_room) = None;
        self.room_order.clear();
        self.sent.send_replace(Vec::new());
        self.home.set_catching_up(false);
        self.view.send_modify(|view| {
            view.pending_confirmations.clear();
            view.settled_confirmations.clear();
        });
        let session = lock(&self.session).take();
        if let Some(session) = session {
            session.stop_sync().await;
        }
    }

    /// 把会话的房间列表、加密状态、同步状态和会话事件接到 ViewModel 上。
    fn observe(self: &Arc<Self>, session: &Arc<MatrixSession>) -> Vec<JoinHandle<()>> {
        let view = self.view.clone();
        let rooms = {
            let session = session.clone();
            let view = view.clone();
            let room_order = self.room_order.clone();
            let primary_bot = self.primary_bot.clone();
            self.runtime.spawn(async move {
                let rooms = match session.rooms().await {
                    Ok(rooms) => rooms,
                    Err(error) => {
                        tracing::warn!("拿不到房间列表：{error}");
                        return;
                    }
                };
                let core = core_account(session.user_id());
                let mut rooms = std::pin::pin!(rooms);
                while let Some(rooms) = rooms.next().await {
                    let mut rows: Vec<_> = rooms
                        .into_iter()
                        .map(|room| room_row(room, &core))
                        .collect();
                    sort_rooms(&mut rows);
                    let bot = lock(&primary_bot).clone();
                    let rows = room_order.replace(rows, bot.as_deref());
                    view.send_modify(|view| view.rooms = rows);
                }
            })
        };
        let encryption = self
            .runtime
            .spawn(recovery::watch(Arc::downgrade(self), session.clone()));
        let sync = {
            let statuses = session.sync_status();
            let view = view.clone();
            self.runtime.spawn(async move {
                let mut statuses = std::pin::pin!(statuses);
                while let Some(status) = statuses.next().await {
                    let sync = match status {
                        SyncStatus::Idle => ChatSync::Idle,
                        SyncStatus::Starting => ChatSync::Starting,
                        SyncStatus::Running => ChatSync::Running,
                        SyncStatus::Failed(error) => ChatSync::Failed {
                            reason: failure(&error),
                            message: error.to_string(),
                        },
                    };
                    view.send_if_modified(|view| {
                        // 刚登录时 SDK 的同步服务还没启动，它报的“未启动”不覆盖“正在启动”。
                        if sync == ChatSync::Idle && view.sync == ChatSync::Starting {
                            return false;
                        }
                        view.sync = sync;
                        true
                    });
                }
            })
        };
        let events = self
            .runtime
            .spawn(session::watch(Arc::downgrade(self), session.clone()));
        let syncing = {
            let session = session.clone();
            self.runtime
                .spawn(async move { session.keep_syncing().await })
        };
        let cards = self.runtime.spawn(pending::follow_cards(
            view,
            session.clone(),
            self.answers.clone(),
            self.card_success.clone(),
        ));
        let catching_up = {
            let updates = session.catching_up();
            let home = self.home.clone();
            self.runtime.spawn(async move {
                let mut updates = std::pin::pin!(updates);
                while let Some(catching_up) = updates.next().await {
                    home.set_catching_up(catching_up);
                }
            })
        };
        vec![rooms, encryption, sync, events, syncing, cards, catching_up]
    }
}

fn invalid(reason: &str) -> CoreError {
    CoreError::InvalidInput {
        reason: reason.to_owned(),
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
