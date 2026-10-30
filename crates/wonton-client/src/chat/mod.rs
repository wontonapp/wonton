//! 聊天：经隧道登录主人的 Matrix 账号，房间列表、时间线、收发消息和端到端加密（ADR-0003、ADR-0005，以及
//! ADR-0009 的“接口：App 登录和使用 Matrix”）。
//!
//! 跟着家里的连接走：本地代理就绪（或者换了）就登录。上次登录过就用存下来的登录状态恢复，不经服务器；否则经
//! 隧道向核心要一次性的登录令牌，用它登录 `matrix.<基础域名>`。所有 Matrix 请求都走本地代理，Tuwunel 只被当成
//! 标准的 Matrix 服务器（ADR-0015）。

mod login;
mod storage;
mod timeline;
mod view;

use std::future::Future;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, Weak};

use futures_util::StreamExt;
use tokio::runtime::Handle;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use wonton_matrix::{
    ChatRoom, Error, MatrixSession, OwnedRoomId, OwnedUserId, SessionConfig, SessionEvent,
    StoreConfig, SyncStatus,
};
use wonton_protocol::{
    Action, ChatAccount, ChatFailure, ChatProblem, ChatSync, ChatViewModel, CoreError,
    DeviceVerification, EncryptionStatus, HomeStatus, HomeViewModel, MediaHandle, ProxySettings,
    PushStatus, RecoveryStatus, SecretString,
};

pub(crate) use login::proxy_url;
pub use timeline::TimelineSubscription;
pub(crate) use view::{failure, summary};

use crate::home::HomeLink;
use storage::{ChatStorage, StoredSession};
use view::{core_error, recovery, room_row, verification};

/// 本设备在 Matrix 服务器上的显示名。
const DEVICE_NAME: &str = "Wonton App";
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
    ) -> Result<Arc<Self>, CoreError> {
        let storage = ChatStorage::open(data_dir)?;
        let view = watch::Sender::new(ChatViewModel {
            account: ChatAccount::SignedOut,
            sync: ChatSync::Idle,
            encryption: EncryptionStatus {
                verification: DeviceVerification::Unknown,
                recovery: RecoveryStatus::Unknown,
                recovery_key: storage.recovery_key()?,
            },
            rooms: Vec::new(),
            problem: None,
            push: PushStatus::NoEndpoint,
        });
        let chat = Arc::new(Self {
            runtime: runtime.clone(),
            home: home.clone(),
            storage,
            view,
            active: tokio::sync::Mutex::new(None),
            session: Mutex::new(None),
            open_room: Mutex::new(None),
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

    /// App 回到前台：登录失败（不是被吊销）或同步停了的话重试。
    pub(crate) fn foregrounded(self: &Arc<Self>) {
        let view = self.view();
        // 还没连上家里、或者正在登录（家里的连接一就绪就会登录）时不插手；被吊销只能重新配对。Android 启动时
        // 也会发一次“回到前台”，正赶上第一次登录，这时再登录一遍会在同一个存储上开两个会话。
        let retry = match &view.account {
            ChatAccount::Failed { reason, .. } => *reason != ChatFailure::Unauthorized,
            ChatAccount::SignedIn { .. } => {
                matches!(view.sync, ChatSync::Idle | ChatSync::Failed { .. })
            }
            ChatAccount::SignedOut | ChatAccount::SigningIn => false,
        };
        if retry && let Some(proxy) = self.home.view().proxy {
            self.runtime.spawn(self.clone().connect(proxy));
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
            Action::SendText { room_id, body } => {
                let body = body.trim().to_owned();
                if body.is_empty() {
                    return Err(invalid("消息不能为空"));
                }
                let chat = self.clone();
                let (_, room_id) = self.target(&room_id)?;
                self.spawn_op("发送消息", async move {
                    chat.room(&room_id).await?.send_text(&body).await
                });
                Ok(())
            }
            Action::SendImage { room_id, image } => {
                if !image.mime_type.starts_with("image/") {
                    return Err(invalid(&format!("不是图片类型：{}", image.mime_type)));
                }
                let chat = self.clone();
                let (_, room_id) = self.target(&room_id)?;
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
                self.spawn_op("发送图片", async move {
                    chat.room(&room_id).await?.send_image(upload).await
                });
                Ok(())
            }
            Action::RetrySend { .. } | Action::CancelSend { .. } => Err(CoreError::NotReady {
                reason: "这个版本的客户端核心不能重发或取消发送".to_owned(),
            }),
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
            Action::SetUpRecovery => self.set_up_recovery(),
            Action::ConfirmRecoveryKeySaved => {
                self.storage.forget_recovery_key()?;
                self.view
                    .send_modify(|view| view.encryption.recovery_key = None);
                Ok(())
            }
            Action::RecoverEncryption { recovery_key } => self.recover(&recovery_key),
            Action::SetPushEndpoint { .. }
            | Action::PushUnavailable { .. }
            | Action::CreateSession { .. }
            | Action::SelectSession { .. }
            | Action::DeleteSession { .. }
            | Action::ConnectHome { .. }
            | Action::PairHome { .. }
            | Action::InviteDevice
            | Action::AppForegrounded
            | Action::SetLanguages { .. }
            | Action::RefreshPlugins
            | Action::RefreshStore { .. }
            | Action::InstallPlugin { .. }
            | Action::UpgradePlugin { .. }
            | Action::UninstallPlugin { .. }
            | Action::DismissPluginTask { .. } => Err(invalid("不是聊天操作")),
        }
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
        Ok(TimelineSubscription::start(&self.runtime, room))
    }

    /// 凭句柄下载媒体原文件（加密附件在本机解密）。
    pub(crate) async fn load_media(&self, media: MediaHandle) -> Result<Vec<u8>, CoreError> {
        let session = self.current()?;
        session
            .download_media(&media.token)
            .await
            .map_err(|e| core_error(&e))
    }

    fn start_direct_chat(self: &Arc<Self>, user_id: &str) -> Result<(), CoreError> {
        let user: OwnedUserId = user_id
            .parse()
            .map_err(|e| invalid(&format!("用户 ID 不合法：{e}")))?;
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

    fn set_up_recovery(self: &Arc<Self>) -> Result<(), CoreError> {
        let session = self.current()?;
        let chat = self.clone();
        self.set_recovery(RecoveryStatus::InProgress);
        self.spawn_op("设置恢复", async move {
            let key = SecretString::new(session.enable_recovery().await?);
            chat.show_recovery_key(key);
            Ok(())
        });
        Ok(())
    }

    /// 用主人输入的恢复密钥恢复。密钥用完即弃，不存。
    fn recover(self: &Arc<Self>, recovery_key: &SecretString) -> Result<(), CoreError> {
        let key = recovery_key.expose().trim().to_owned();
        if key.is_empty() {
            return Err(invalid("恢复密钥不能为空"));
        }
        let session = self.current()?;
        self.set_recovery(RecoveryStatus::InProgress);
        self.spawn_op("用恢复密钥恢复", async move {
            session.recover(&key).await
        });
        Ok(())
    }

    /// 把刚生成的恢复密钥给主人看，直到主人确认已经另外保存。先存到本机，App 在那之前被关掉，下次启动还能看到；
    /// 存不下就如实报出来，主人要趁现在抄下。
    fn show_recovery_key(&self, key: SecretString) {
        let saved = self.storage.save_recovery_key(&key);
        self.view.send_modify(|view| {
            view.encryption.recovery_key = Some(key);
            if let Err(error) = saved {
                view.problem = Some(ChatProblem {
                    reason: ChatFailure::Upstream,
                    message: format!("恢复密钥没能存到本机，请现在就抄下来：{error}"),
                });
            }
        });
    }

    fn set_recovery(&self, status: RecoveryStatus) {
        self.view
            .send_modify(|view| view.encryption.recovery = status);
    }

    /// 在后台执行一个操作，失败时把原因放进 `problem`。
    fn spawn_op(
        &self,
        what: &'static str,
        op: impl Future<Output = Result<(), Error>> + Send + 'static,
    ) {
        let view = self.view.clone();
        self.runtime.spawn(async move {
            if let Err(error) = op.await {
                tracing::info!("{what}失败：{error}");
                view.send_modify(|view| {
                    view.problem = Some(ChatProblem {
                        reason: failure(&error),
                        message: format!("{what}失败：{error}"),
                    });
                });
            }
        });
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
        } else if reconnected && matches!(link.view().account, ChatAccount::Failed { .. }) {
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
        if let Some(old) = active.take() {
            self.forget_current().await;
            drop(old);
        }
        self.view
            .send_modify(|view| view.account = ChatAccount::SigningIn);
        let session = match self.sign_in(&proxy).await {
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
        let session = lock(&self.session).take();
        if let Some(session) = session {
            session.stop_sync().await;
        }
    }

    async fn sign_in(&self, proxy: &ProxySettings) -> Result<MatrixSession, (ChatFailure, String)> {
        let config = |homeserver_url: String| SessionConfig {
            homeserver_url,
            device_name: DEVICE_NAME.to_owned(),
            store: StoreConfig::Sqlite {
                path: self.storage.store_dir(),
                passphrase: None,
            },
            proxy: Some(login::proxy_url(proxy)),
        };
        let storage_failure = |e: CoreError| (ChatFailure::Upstream, e.to_string());
        let stored = self.storage.session().map_err(storage_failure)?;
        // 登录状态要和 SQLite 存储一起用：存储没了还恢复，SDK 会给已有的设备 ID 生成新的设备密钥。存储不在就
        // 重新登录一台新设备。
        if let Some(stored) = stored.filter(|_| self.storage.has_store()) {
            return MatrixSession::restore(&config(stored.homeserver_url), stored.session)
                .await
                .map_err(|e| (failure(&e), format!("恢复 Matrix 会话失败：{e}")));
        }
        let core_url = self.home.core_url().ok_or((
            ChatFailure::NotReady,
            "还没有家里服务器的连接信息".to_owned(),
        ))?;
        let token = login::login_token(&core_url, proxy, login::LOGIN_TOKEN_TIMEOUT).await?;
        // 没有登录状态时，本地存储里的旧设备密钥属于别的设备，新登录用不上。
        self.storage.forget_session().map_err(storage_failure)?;
        let session =
            MatrixSession::login_with_jwt(&config(token.homeserver_url.clone()), &token.token)
                .await
                .map_err(|e| (failure(&e), format!("登录 Matrix 失败：{e}")))?;
        if session.user_id().as_str() != token.user_id {
            tracing::warn!(expected = %token.user_id, actual = %session.user_id(), "登录得到的用户和核心说的不一样");
        }
        self.save(&session, token.homeserver_url)
            .map_err(storage_failure)?;
        Ok(session)
    }

    fn save(&self, session: &MatrixSession, homeserver_url: String) -> Result<(), CoreError> {
        let Some(saved) = session.saved() else {
            return Err(CoreError::Storage {
                reason: "登录后取不到登录状态".to_owned(),
            });
        };
        self.storage.save_session(&StoredSession {
            homeserver_url,
            session: saved,
        })
    }

    /// 把会话的房间列表、加密状态、同步状态和会话事件接到 ViewModel 上。
    fn observe(self: &Arc<Self>, session: &Arc<MatrixSession>) -> Vec<JoinHandle<()>> {
        let view = self.view.clone();
        let rooms = {
            let session = session.clone();
            let view = view.clone();
            self.runtime.spawn(async move {
                let rooms = match session.rooms().await {
                    Ok(rooms) => rooms,
                    Err(error) => {
                        tracing::warn!("拿不到房间列表：{error}");
                        return;
                    }
                };
                let mut rooms = std::pin::pin!(rooms);
                while let Some(rooms) = rooms.next().await {
                    let rows = rooms.into_iter().map(room_row).collect();
                    view.send_modify(|view| view.rooms = rows);
                }
            })
        };
        let encryption = {
            let states = session.encryption_states();
            let view = view.clone();
            self.runtime.spawn(async move {
                let mut states = std::pin::pin!(states);
                while let Some(state) = states.next().await {
                    view.send_modify(|view| {
                        view.encryption.verification = verification(state.verification);
                        view.encryption.recovery = recovery(state.recovery);
                    });
                }
            })
        };
        let sync = {
            let statuses = session.sync_status();
            let view = view.clone();
            self.runtime.spawn(async move {
                let mut statuses = std::pin::pin!(statuses);
                while let Some(status) = statuses.next().await {
                    let sync = match status {
                        SyncStatus::Idle => ChatSync::Idle,
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
            .spawn(watch_session(Arc::downgrade(self), session.clone()));
        vec![rooms, encryption, sync, events]
    }
}

/// 令牌刷新后存下新的登录状态；设备被吊销时如实显示，并删掉用不上的登录状态和本地存储。
async fn watch_session(chat: Weak<ChatLink>, session: Arc<MatrixSession>) {
    let mut events = std::pin::pin!(session.session_events());
    while let Some(event) = events.next().await {
        let Some(chat) = chat.upgrade() else { return };
        match event {
            SessionEvent::TokensRefreshed => {
                let saved = chat.storage.session().and_then(|stored| match stored {
                    Some(stored) => chat.save(&session, stored.homeserver_url),
                    None => Ok(()),
                });
                if let Err(error) = saved {
                    tracing::warn!("刷新后的 Matrix 令牌没能存下：{error}");
                }
            }
            SessionEvent::TokenRejected { soft_logout } => {
                tracing::warn!(soft_logout, "Matrix 服务器不再接受本设备的令牌");
                if !soft_logout {
                    session.stop_sync().await;
                    if let Err(error) = chat.storage.forget_session() {
                        tracing::warn!("删除失效的 Matrix 登录状态失败：{error}");
                    }
                }
                chat.view.send_modify(|view| {
                    view.account = ChatAccount::Failed {
                        reason: ChatFailure::Unauthorized,
                        message: "这台设备在 Matrix 上已被吊销，需要主人重新配对".to_owned(),
                    };
                    view.sync = ChatSync::Idle;
                });
            }
        }
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
