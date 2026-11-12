//! 聊天：经隧道登录主人的 Matrix 账号，房间列表、时间线、收发消息和端到端加密（ADR-0003、ADR-0005，以及
//! ADR-0009 的“接口：App 登录和使用 Matrix”）。
//!
//! 跟着家里的连接走：本地代理就绪（或者换了）就登录。上次登录的是同一台服务器就用存下来的登录状态恢复，不经服务器；
//! 否则经隧道向核心要一次性的登录令牌，用它登录 `matrix.<基础域名>`。所有 Matrix 请求都走本地代理，Tuwunel 只被当成
//! 标准的 Matrix 服务器（ADR-0015）。
//!
//! 外部 Matrix 账号的槽位（ADR-0051）不跟着家里的连接走：直连那个账号的 homeserver，用登录时
//! （`matrix_login`）存进槽位的登录状态和存储恢复；登录状态没了只能重新登录那个账号。

mod accessors;
mod agent_run;
mod ask;
mod avatar_files;
mod background;
mod bot_activity;
mod bot_poll;
mod bot_profile;
mod bot_typing;
mod bots;
mod confirmation;
mod connect;
mod contacts;
mod display_name;
mod draft;
mod follow_home;
mod forward;
mod handoff;
mod installs;
mod layout;
mod links;
mod login;
mod menu;
mod onboarding;
mod ops;
mod pending;
mod read_mark;
pub(crate) mod recovery;
mod reply;
mod rich;
mod room_rows;
mod send;
mod session;
mod sign_in;
mod storage;
mod timeline;
mod verification;
mod view;

use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::Duration;

use futures_util::StreamExt;
#[cfg(feature = "test-hooks")]
use futures_util::future::BoxFuture;
use tokio::runtime::Handle;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use wonton_l10n::l10n;
use wonton_matrix::{ChatRoom, Error, MatrixSession, OwnedRoomId, OwnedUserId, RoomId, SyncStatus};
use wonton_protocol::{
    Action, ChatAccount, ChatFailure, ChatSync, ChatViewModel, CoreError, DeviceVerification,
    EncryptionStatus, MediaHandle, PushMethod, PushStatus, RecoveryNextStep, RecoveryStatus,
    RoomPanel, TimelineAction, TimelineScroll, TimelineTarget,
};

pub use bots::bot_current_avatar;
pub(crate) use confirmation::{PushedCard, core_account, pushed_card};
pub(crate) use links::{LinkTargets, matrix_target};
pub(crate) use login::proxy_url;
pub use timeline::TimelineSubscription;
pub(crate) use view::{failure, summary};

use crate::home::HomeLink;
use crate::l10n::{CATALOG, language};
use crate::scope::reuse;
use confirmation::{Answers, Cards};
use draft::DraftWrite;
use layout::Layouts;
pub(crate) use storage::{ChatStorage, StoredSession};
use view::core_error;

/// 向前翻页一次最多加载的事件数。
const PAGE: u16 = 30;

/// 发消息等多久：核心回 502 时 matrix-sdk 自己要十几分钟才放弃，期限盖住它；被放弃的那条还会留在 SDK 的发送
/// 队列里按自己的退避重试，重发的消息排在后面，30 秒撑不住慢机器，给满一分钟。
const SEND_TIMEOUT: Duration = Duration::from_secs(60);

pub(crate) struct ChatLink {
    runtime: Handle,
    home: Arc<HomeLink>,
    /// 本作用域的槽位号（`ServerRow::id`）：后台通知的 `server_id` 填它，壳点开通知按它切服务器；
    /// 不是 home 里的 endpoint ID，那是配对键，槽位列表里没有。
    slot: String,
    storage: ChatStorage,
    view: watch::Sender<ChatViewModel>,
    active: tokio::sync::Mutex<Option<Active>>,
    session: Mutex<Option<Arc<MatrixSession>>>,
    session_out: watch::Sender<Option<Arc<MatrixSession>>>,
    room_panel_out: watch::Sender<Option<(OwnedRoomId, RoomPanel)>>,
    /// 订阅着时间线的房间。同一时间只打开一个房间。
    open_room: Mutex<Option<(OwnedRoomId, Arc<ChatRoom>)>>,
    /// 最近一次订阅的时间线里每条消息的菜单，转发、撤回只认这里有的（[`menu`]）。
    open_menus: Mutex<Option<(OwnedRoomId, Arc<Mutex<menu::Menus>>)>>,
    /// 打开的房间的时间线订阅，用来把壳报告的滚动位置送进去。订阅没了（壳重开、房间关掉）就升级不出来。
    open_timeline: Mutex<Option<(OwnedRoomId, Weak<TimelineSubscription>)>>,
    /// 这台设备回答确认卡片的进展，时间线的订阅跟着它更新卡片。
    answers: watch::Sender<Answers>,
    /// 本会话里亲眼看到从没结束变成成功的插件 ID：插件列表多半还是旧的，补取的逻辑在插件那头。
    card_success: CardSuccess,
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
    /// 聊天列表最上面的安装占位行（ADR-0067）。
    installs: Arc<installs::Installs>,
    /// 聊天显示方式的账号数据（ADR-0060），只读给房间和 Bot 资料页合成用。
    layouts: Arc<Layouts>,
    bots: Arc<bots::Bots>,
    plugins: Arc<crate::plugins::PluginsLink>,
    avatar_files: avatar_files::AvatarFiles,
    /// 收新消息的方式（ADR-0003）：保持连接时后台同步到的新消息要弹通知。
    method: watch::Receiver<PushMethod>,
    /// 排队的草稿写入，见 [`draft::write_drafts`]。
    draft_writes: mpsc::UnboundedSender<DraftWrite>,
    /// App 在后台：保持连接下同步到的该通知的消息收集进 `background_notifications`。
    background: AtomicBool,
    verification: Mutex<Option<verification::Control>>,
    /// “添加设备”页开着、还能自动接受一个新设备的确认请求（ADR-0087）。
    handoff_armed: AtomicBool,
    relogin: connect::Relogin,
    external: bool,
}

/// 卡片变成成功时就地调用（插件 ID），在卡片的情况发出去之前：ViewModel 里卡片“已经不用回答”时，补不补取已经定了。
#[cfg(not(feature = "test-hooks"))]
pub(crate) type CardSuccess = Arc<dyn Fn(&str) + Send + Sync>;

/// 开着测试钩子（feature `test-hooks`）时，决定是个可等待的 future：钩子先报到、等放行，决定才继续，
/// settled 也发不出去。见 `test_hooks`。
#[cfg(feature = "test-hooks")]
pub(crate) type CardSuccess = Arc<dyn for<'a> Fn(&'a str) -> BoxFuture<'a, ()> + Send + Sync>;

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

/// matrix-sdk 的 sqlite 连接池释放连接时要在 tokio 运行时的上下文里（`spawn_blocking`），没有就 panic，接着释放下一个
/// 连接时再 panic，进程直接中止。最后一个引用可能在壳的线程上释放：换服务器时旧作用域在 `dispatch` 里释放（实测
/// Android 上点“添加服务器”闪退），临时作用域在壳驱动的 async 方法里释放。所以在自己运行时的上下文里放掉会话、Bot 那边
/// 存的会话和打开的房间；那个运行时这时多半已经停了，`spawn_blocking` 不再执行，连接就地关闭。Bot 那边只放会话，不走
/// `set_session`：它会改 ViewModel，而发送端这时已经交给了新服务器的作用域。
impl Drop for ChatLink {
    fn drop(&mut self) {
        let _context = self.runtime.enter();
        lock(&self.open_room).take();
        lock(&self.open_timeline).take();
        lock(&self.session).take();
        self.bots.session.send_replace(None);
        self.active.get_mut().take();
    }
}

/// ChatLink 对外的通道：scope 建好，chat 往里喂，tabs 等拿去用。
pub(crate) struct ChatChannels {
    /// ViewModel 发送端，换服务器时给壳接着订阅用。
    pub(crate) view: Option<watch::Sender<ChatViewModel>>,
    /// 推送方式。
    pub(crate) method: watch::Receiver<PushMethod>,
    /// 登录着的会话。
    pub(crate) session_out: watch::Sender<Option<Arc<MatrixSession>>>,
    /// 打开的房间的面板（房间账号数据）。
    pub(crate) room_panel_out: watch::Sender<Option<(OwnedRoomId, RoomPanel)>>,
    /// 外部 Matrix 账号的槽位（ADR-0051）：不跟着家里的连接走，直连 homeserver，用登录时存下的状态恢复。
    pub(crate) external: bool,
}

impl ChatLink {
    pub(crate) fn new(
        runtime: Handle,
        home: Arc<HomeLink>,
        slot: &str,
        data_dir: &Path,
        card_success: CardSuccess,
        plugins: Arc<crate::plugins::PluginsLink>,
        channels: ChatChannels,
    ) -> Result<Arc<Self>, CoreError> {
        let ChatChannels {
            view,
            method,
            session_out,
            room_panel_out,
            external,
        } = channels;
        let storage = ChatStorage::open(data_dir)?;
        let mut encryption = EncryptionStatus {
            verification: DeviceVerification::Unknown,
            recovery: RecoveryStatus::Unknown,
            recovery_key: storage.recovery_key()?,
            backup_reminder: None,
            can_verify_with_other_device: false,
            can_reset: false,
            recovery_next_step: RecoveryNextStep::None,
            verification_flow: None,
            key_handoff: None,
        };
        recovery::refresh(&mut encryption);
        let view = reuse(
            view,
            ChatViewModel {
                chat_layout_default: Default::default(),
                account: ChatAccount::SignedOut,
                sync: ChatSync::Idle,
                encryption,
                installing_bots: Vec::new(),
                rooms: Vec::new(),
                problem: None,
                push: PushStatus::NoEndpoint,
                pending_confirmations: Vec::new(),
                settled_confirmations: Vec::new(),
                background_notifications: Vec::new(),
                bots: Vec::new(),
            },
        );
        let installs = installs::Installs::new(plugins.clone(), view.clone());
        let layouts = Layouts::new();
        let bots = bots::Bots::new(
            runtime.clone(),
            home.clone(),
            plugins.clone(),
            view.clone(),
            layouts.clone(),
        );
        let (draft_writes, draft_receiver) = mpsc::unbounded_channel();
        let chat = Arc::new(Self {
            runtime: runtime.clone(),
            home: home.clone(),
            slot: slot.to_owned(),
            storage,
            view,
            active: tokio::sync::Mutex::new(None),
            session: Mutex::new(None),
            session_out,
            room_panel_out,
            open_room: Mutex::new(None),
            open_menus: Mutex::new(None),
            open_timeline: Mutex::new(None),
            answers: watch::Sender::default(),
            card_success,
            resetting_recovery: AtomicBool::default(),
            primary_bot: Arc::new(Mutex::new(None)),
            room_order: Arc::new(onboarding::RoomOrder::new()),
            sent: watch::Sender::new(Vec::new()),
            display_name: Mutex::new(None),
            installs,
            layouts,
            bots,
            plugins,
            avatar_files: avatar_files::AvatarFiles::new(data_dir),
            method,
            draft_writes,
            background: AtomicBool::new(false),
            verification: Mutex::new(None),
            handoff_armed: AtomicBool::new(false),
            relogin: connect::Relogin::default(),
            external,
        });
        runtime.spawn(draft::write_drafts(Arc::downgrade(&chat), draft_receiver));
        if external {
            runtime.spawn(chat.clone().connect(connect::Target::External));
        } else {
            runtime.spawn(follow_home::run(Arc::downgrade(&chat), home.subscribe()));
        }
        Ok(chat)
    }

    /// App 回到前台或家里的连接恢复：登录失败（不是被吊销）或正在等着重连的话马上重新登录；已经登录的接着用同一个
    /// 会话，同步和发送队列马上恢复。换会话的话，打开着的房间还订阅着旧会话，再也收不到更新。
    pub(crate) fn foregrounded(self: &Arc<Self>) {
        let view = self.view();
        // 还没连上家里、或者正在登录（家里的连接一就绪就会登录）时不插手；被吊销只能重新配对。Android 启动时
        // 也会发一次“回到前台”，正赶上第一次登录，这时再登录一遍会在同一个存储上开两个会话。
        match &view.account {
            ChatAccount::Failed {
                reason: ChatFailure::Unauthorized,
                ..
            }
            | ChatAccount::SignedOut
            | ChatAccount::SigningIn => {}
            // 正在等着重连的不再等，马上登录。
            ChatAccount::Reconnecting { .. } | ChatAccount::Failed { .. } => {
                let target = match self.home.view().proxy {
                    _ if self.external => Some(connect::Target::External),
                    Some(proxy) => Some(connect::Target::Home(proxy)),
                    None => None,
                };
                if let Some(target) = target {
                    self.runtime.spawn(self.clone().connect(target));
                }
            }
            ChatAccount::SignedIn { .. } => {
                if let Some(session) = lock(&self.session).clone() {
                    session.resume();
                }
            }
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
                self.spawn_op("chat-op-join-failed", async move {
                    session.join_room(&room_id).await
                });
                Ok(())
            }
            Action::LeaveRoom { room_id } => {
                let (session, room_id) = self.target(&room_id)?;
                self.spawn_op("chat-op-leave-failed", async move {
                    session.leave_room(&room_id).await
                });
                Ok(())
            }
            Action::StartDirectChat { user_id } => self.start_direct_chat(&user_id),
            Action::Timeline { action } => self.timeline_action(action),
            Action::AnswerAgentChoice {
                room_id,
                item_id,
                key,
            } => self.answer_agent_choice(&room_id, &item_id, &key),
            Action::ResetRecovery => self.reset_recovery(),
            Action::ConfirmRecoveryKeySaved => self.confirm_recovery_key_saved(),
            Action::RecoverEncryption { recovery_key } => self.recover(&recovery_key),
            action @ (Action::StartDeviceVerification
            | Action::AcceptDeviceVerification { .. }
            | Action::ConfirmVerification { .. }
            | Action::DenyVerification { .. }
            | Action::CancelDeviceVerification { .. }) => self.verification_action(action),
            Action::AnswerConfirmation {
                id,
                answer,
                choice_id,
                secrets,
                settings,
            } => self.answer_confirmation(id, answer, choice_id, secrets, settings),
            Action::StopConfirmationWindow { id } => self.stop_confirmation_window(id),
            Action::SetChatLayout { target, choice } => self.set_chat_layout(target, choice),
            _ => Err(invalid(&l10n!(CATALOG, language(), "chat-invalid-action"))),
        }
    }

    /// 旁聊时间线（`TimelineTarget::thread_root` 不为空）还没实现（接口先合并，ADR-0093 §5）：
    /// 调用方拿到明确的“还不能用”，不是空的成功。
    fn side_chat_not_ready() -> CoreError {
        CoreError::NotReady {
            reason: l10n!(CATALOG, language(), "side-chat-not-ready"),
        }
    }

    /// 拆 [`TimelineTarget`]：主时间线回房间 ID；带话题根的报还不能用。
    fn timeline_target(&self, target: &TimelineTarget) -> Result<String, CoreError> {
        if target.thread_root.is_some() {
            return Err(Self::side_chat_not_ready());
        }
        Ok(target.room_id.clone())
    }

    /// 按时间线（主时间线或旁聊，ADR-0093 §5）分发的操作。实现和主时间线共用；旁聊路径在
    /// [`Self::timeline_target`] 处统一挡下。
    fn timeline_action(self: &Arc<Self>, action: TimelineAction) -> Result<(), CoreError> {
        match action {
            TimelineAction::SendText {
                target,
                body,
                reply_to,
            } => {
                let room_id = self.timeline_target(&target)?;
                self.send_text(&room_id, &body, reply_to.as_deref())
            }
            TimelineAction::SendImage {
                target,
                image,
                reply_to,
            } => {
                let room_id = self.timeline_target(&target)?;
                self.send_image(&room_id, image, reply_to.as_deref())
            }
            TimelineAction::ForwardMessage {
                target,
                item_id,
                to_room_id,
            } => {
                let room_id = self.timeline_target(&target)?;
                self.forward_message(&room_id, &item_id, &to_room_id)
            }
            TimelineAction::RedactMessage { target, item_id } => {
                let room_id = self.timeline_target(&target)?;
                self.redact_message(&room_id, &item_id)
            }
            TimelineAction::RetrySend { target, item_id } => {
                let room_id = self.timeline_target(&target)?;
                let room = self.opened(&room_id)?;
                let sent = self.sent.clone();
                self.spawn_op_with_success(
                    "chat-op-resend-failed",
                    async move { room.retry_send(&item_id).await },
                    move || onboarding::record_sent(&sent, room_id),
                );
                Ok(())
            }
            TimelineAction::CancelSend { target, item_id } => {
                let room_id = self.timeline_target(&target)?;
                let room = self.opened(&room_id)?;
                self.spawn_op("chat-op-cancel-send-failed", async move {
                    room.cancel_send(&item_id).await
                });
                Ok(())
            }
            TimelineAction::PaginateBackwards { target } => {
                let room_id = self.timeline_target(&target)?;
                let room = self.opened(&room_id)?;
                self.spawn_op("chat-op-paginate-failed", async move {
                    room.paginate_backwards(PAGE).await.map(drop)
                });
                Ok(())
            }
            TimelineAction::SetTimelineScroll { target, scroll } => {
                let room_id = self.timeline_target(&target)?;
                self.set_timeline_scroll(&room_id, scroll)
            }
            TimelineAction::MarkRead { target } => {
                let room_id = self.timeline_target(&target)?;
                let chat = self.clone();
                let (_, room_id) = self.target(&room_id)?;
                self.spawn_op("chat-op-mark-read-failed", async move {
                    chat.room(&room_id).await?.mark_as_read().await.map(drop)
                });
                Ok(())
            }
            TimelineAction::SetDraft {
                target,
                body,
                reply_to,
            } => {
                let room_id = self.timeline_target(&target)?;
                self.set_draft(&room_id, &body, reply_to.as_deref())
            }
            // 附件区的操作由 `ClientCore::dispatch` 交给附件区（ADR-0094），不会到这里。
            TimelineAction::SendComposer { .. }
            | TimelineAction::RemoveAttachment { .. }
            | TimelineAction::RetryAttachment { .. }
            | TimelineAction::RetryOutgoing { .. }
            | TimelineAction::CancelOutgoing { .. } => {
                Err(invalid(&l10n!(CATALOG, language(), "chat-invalid-action")))
            }
        }
    }

    fn confirm_recovery_key_saved(&self) -> Result<(), CoreError> {
        self.storage.forget_recovery_key()?;
        self.view.send_modify(|view| {
            view.encryption.recovery_key = None;
            recovery::refresh(&mut view.encryption);
        });
        Ok(())
    }

    /// 订阅一个已加入房间的时间线。打开另一个房间会关掉前一个的订阅。
    ///
    /// `room_view` 是 tabs 合成的房间面板视图（面板页、上次看的那页、入口有没有被盖住），每批时间线增量
    /// 都带上当前值；变了也发一批（增量为空）。
    pub(crate) async fn open_timeline(
        self: &Arc<Self>,
        room_id: &str,
        room_view: watch::Receiver<Option<Arc<crate::tabs::RoomPanelView>>>,
    ) -> Result<Arc<TimelineSubscription>, CoreError> {
        let (session, room_id) = self.target(room_id)?;
        if *self.bots.foreground.borrow() {
            self.bots.refresh_all();
        }
        let room = Arc::new(
            session
                .open_room(&room_id)
                .await
                .map_err(|e| core_error(&e))?,
        );
        *lock(&self.open_room) = Some((room_id.clone(), room.clone()));
        let menus = Arc::new(Mutex::new(menu::Menus::default()));
        *lock(&self.open_menus) = Some((room_id.clone(), menus.clone()));
        self.publish_room_panel(&session, &room_id).await;
        // 链接的类别要认插件页，插件列表还一次没取过时先取一次（取过或正在取就不重复，免得打乱别处的节奏）。
        let plugins = self.plugins.view();
        if plugins.installed.is_none() && !plugins.refreshing {
            self.plugins.refresh_installed();
        }
        let cards = Cards::new(session.user_id(), self.answers.subscribe());
        let subscription = TimelineSubscription::start(
            &self.runtime,
            room,
            cards,
            self.plugins.clone(),
            menus,
            room_view,
            self.view.subscribe(),
        );
        *lock(&self.open_timeline) = Some((room_id, Arc::downgrade(&subscription)));
        Ok(subscription)
    }

    /// 读打开着的房间的面板（房间账号数据），交给 tabs 合成给壳的视图。
    ///
    /// 读不了时按没有面板开：房间照常打开，主人编辑面板时写不进去，那里会报原因。
    pub(crate) async fn publish_room_panel(&self, session: &Arc<MatrixSession>, room_id: &RoomId) {
        let panel = match session.room_panel(room_id).await {
            Ok(content) => content
                .and_then(|content| serde_json::from_value(content.0.into()).ok())
                .unwrap_or_default(),
            Err(error) => {
                tracing::warn!("could not read the room panel of {room_id}: {error}");
                RoomPanel::default()
            }
        };
        self.room_panel_out
            .send_replace(Some((room_id.to_owned(), panel)));
    }

    /// 壳报告列表滚到了哪儿：交给这个房间现在的时间线订阅。
    fn set_timeline_scroll(&self, room_id: &str, scroll: TimelineScroll) -> Result<(), CoreError> {
        let subscription = lock(&self.open_timeline)
            .as_ref()
            .filter(|(id, _)| id.as_str() == room_id)
            .and_then(|(_, subscription)| subscription.upgrade());
        match subscription {
            Some(subscription) => {
                subscription.set_scroll(scroll);
                Ok(())
            }
            None => Err(CoreError::NotReady {
                reason: l10n!(
                    CATALOG,
                    language(),
                    "chat-room-not-subscribed",
                    room = room_id
                ),
            }),
        }
    }

    /// 凭句柄下载媒体原文件（加密附件在本机解密）。
    pub(crate) async fn load_media(&self, media: MediaHandle) -> Result<Vec<u8>, CoreError> {
        let session = self.current()?;
        session
            .download_media(&media.token)
            .await
            .map_err(|e| core_error(&e))
    }

    pub(crate) async fn avatar_file(&self, media: MediaHandle) -> Result<String, CoreError> {
        let session = lock(&self.session).clone();
        self.avatar_files.get(session, media).await
    }

    /// 发一条文字，不打开房间：传完文件发链接时，主人可能已经在看别的房间了，打开这个房间会打断那边的时间线。
    pub(crate) async fn send_text_unopened(
        &self,
        room_id: &str,
        body: &str,
    ) -> Result<(), CoreError> {
        let (session, room_id) = self.target(room_id)?;
        // Tuwunel 停着时核心回 502，matrix-sdk 自己要十几分钟才放弃，发链接的传输一直显示在传：给发送设个期限。
        tokio::time::timeout(SEND_TIMEOUT, session.send_text_to(&room_id, body))
            .await
            .map_err(|_| CoreError::TransportUnavailable {
                reason: l10n!(CATALOG, language(), "chat-send-no-response"),
            })?
            .map_err(|e| core_error(&e))?;
        onboarding::record_sent(&self.sent, room_id.to_string());
        Ok(())
    }

    /// App 重启后马上要发消息时用（传完文件发链接）：等登录出结果。登录好了返回真；停在要用户处理的失败上、
    /// 或者一直没动静过了期限，返回假。
    pub(crate) async fn signed_in_within(&self, wait: Duration) -> bool {
        send::signed_in_within(self.subscribe(), wait).await
    }

    fn start_direct_chat(self: &Arc<Self>, user_id: &str) -> Result<(), CoreError> {
        let user: OwnedUserId = user_id
            .parse()
            .map_err(|_| invalid(&l10n!(CATALOG, language(), "chat-invalid-user-id")))?;
        let session = self.current()?;
        if session.direct_chat_with(&user).is_some() {
            return Ok(());
        }
        // 和 Bot 的主房间一样端到端加密：推送只带房间和事件 ID，正文在手机上解密（ADR-0003 的“设计：加密推送”）。
        self.spawn_op("chat-op-direct-chat-failed", async move {
            session.create_direct_chat(&user, true).await.map(drop)
        });
        Ok(())
    }

    /// 推送的状态由 `push` 模块维护，放在聊天的 ViewModel 里。
    pub(crate) fn set_push(&self, status: PushStatus) {
        self.view.send_modify(|view| view.push = status);
    }

    pub(crate) fn current(&self) -> Result<Arc<MatrixSession>, CoreError> {
        lock(&self.session)
            .clone()
            .ok_or_else(|| CoreError::NotReady {
                reason: l10n!(CATALOG, language(), "chat-not-signed-in"),
            })
    }

    fn target(&self, room_id: &str) -> Result<(Arc<MatrixSession>, OwnedRoomId), CoreError> {
        let room_id = room_id.parse::<OwnedRoomId>().map_err(|e| {
            invalid(&l10n!(
                CATALOG,
                language(),
                "chat-invalid-room-id",
                error = e.to_string()
            ))
        })?;
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
            return Err(invalid(&l10n!(CATALOG, language(), "chat-read-only-room")));
        }
        self.target(room_id)
    }

    fn opened(&self, room_id: &str) -> Result<Arc<ChatRoom>, CoreError> {
        match &*lock(&self.open_room) {
            Some((id, room)) if id.as_str() == room_id => Ok(room.clone()),
            _ => Err(CoreError::NotReady {
                reason: l10n!(
                    CATALOG,
                    language(),
                    "chat-room-not-subscribed",
                    room = room_id
                ),
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

impl ChatLink {
    async fn forget_current(&self) {
        self.bots.set_session(None);
        self.session_out.send_replace(None);
        self.room_panel_out.send_replace(None);
        *lock(&self.verification) = None;
        *lock(&self.open_room) = None;
        *lock(&self.open_menus) = None;
        *lock(&self.open_timeline) = None;
        self.room_order.clear();
        self.sent.send_replace(Vec::new());
        self.home.set_catching_up(false);
        self.view.send_modify(|view| {
            recovery::forgot_session(&mut view.encryption);
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
        let background = self.observe_background(session);
        let rooms = self.observe_rooms(session);
        let layouts = layout::follow(self, session).expect("the chat link is still alive here");
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
        let mut tasks = vec![
            background,
            rooms,
            layouts,
            encryption,
            sync,
            events,
            syncing,
            cards,
            catching_up,
        ];
        tasks.extend(self.verification_tasks(session));
        tasks
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
