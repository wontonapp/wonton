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
mod bot_records;
mod bot_typing;
mod bot_web;
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
mod plugin_status;
mod previews;
mod read_mark;
pub(crate) mod recovery;
mod reply;
mod rich;
mod room_link;
mod room_rows;
mod send;
mod session;
mod side_chat;
mod sign_in;
mod storage;
mod target;
mod timeline;
mod timeline_previews;
mod util;
mod verification;
mod view;

use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::StreamExt;
#[cfg(feature = "test-hooks")]
use futures_util::future::BoxFuture;
use tokio::runtime::Handle;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use wonton_l10n::l10n;
use wonton_matrix::{
    ChatRoom, Error, MatrixSession, OwnedRoomId, OwnedUserId, RoomId, SyncStatus, UserId,
};
use wonton_protocol::{
    Action, CORE_LOCALPART, ChatAccount, ChatFailure, ChatSync, ChatViewModel, ConfirmationView,
    ConfirmationsViewModel, CoreError, DeviceVerification, EncryptionStatus, MediaHandle,
    PushMethod, PushStatus, RecoveryNextStep, RecoveryStatus, RoomPanel, TimelineAction,
    TimelineScroll, TimelineTarget,
};

pub use bots::bot_current_avatar;
pub(crate) use confirmation::fetch_card;
pub(crate) use links::{LinkTargets, matrix_target};
pub(crate) use login::proxy_url;
pub use timeline::{TimelineSource, TimelineSubscription};
pub(crate) use view::{failure, summary, unread_total};

use crate::home::HomeLink;
use crate::l10n::{CATALOG, language};
use crate::scope::reuse;
use confirmation::Answers;
use draft::DraftWrite;
use layout::Layouts;

pub(crate) use storage::{ChatStorage, StoredSession};
pub(crate) use target::TimelineKey;
use util::{invalid, lock};
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
    /// 订阅着的时间线：同一时间只开一个房间，主对话和一个旁聊可以同时开着（[`target`]）。
    open: Mutex<target::OpenTimelines>,
    opening: tokio::sync::Mutex<()>,
    /// 各房间的旁聊支持声明（ADR-0093 §1），按信任规则核对过。
    side_chat_support: Arc<side_chat::Support>,
    /// 这台设备回答确认卡片的进展，时间线的订阅跟着它更新卡片。
    answers: watch::Sender<Answers>,
    /// “等你确认”列表（ADR-0016）：长轮询核心的确认接口得到的视图，壳订阅它。换服务器时给壳接着订阅用，
    /// 所以从通道进来。
    confirmations: watch::Sender<ConfirmationsViewModel>,
    /// 会话里能显示的卡片（等回答的，加上这个进程里看着结束的），按 `origin.room_id` 由时间线的出口过滤。
    room_cards: watch::Sender<Arc<Vec<ConfirmationView>>>,
    /// `Action::RetryConfirmations` 放行长轮询的退避等待。
    retry_confirmations: Arc<tokio::sync::Notify>,
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
    /// App 锁（ADR-0102）：锁着时弹出的通知不带内容。
    app_lock: crate::app_lock::AppLockLink,
    /// 锁屏上隐藏内容的类别（ADR-0101 §5）：后台通知照它设系统的可见性。
    lock_screen: watch::Receiver<crate::lock_screen::Hidden>,
    /// 排队的草稿写入，见 [`draft::write_drafts`]。
    draft_writes: mpsc::UnboundedSender<DraftWrite>,
    /// 链接预览的按网址缓存和点掉的记录（ADR-0065），发送框和时间线共用。
    previews: previews::Previews,
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
        lock(&self.open).clear();
        lock(&self.session).take();
        self.bots.session.send_replace(None);
        self.active.get_mut().take();
    }
}

/// ChatLink 对外的通道：scope 建好，chat 往里喂，tabs 等拿去用。
pub(crate) struct ChatChannels {
    /// ViewModel 发送端，换服务器时给壳接着订阅用。
    pub(crate) view: Option<watch::Sender<ChatViewModel>>,
    /// “等你确认”列表的发送端，同上。
    pub(crate) confirmations: Option<watch::Sender<ConfirmationsViewModel>>,
    /// 推送方式。
    pub(crate) method: watch::Receiver<PushMethod>,
    /// App 锁，锁着时通知藏内容。
    pub(crate) app_lock: crate::app_lock::AppLockLink,
    /// 锁屏上隐藏内容的类别（ADR-0101 §5），通知照它设系统的可见性。
    pub(crate) lock_screen: watch::Receiver<crate::lock_screen::Hidden>,
    /// 登录着的会话。
    pub(crate) session_out: watch::Sender<Option<Arc<MatrixSession>>>,
    /// 打开的房间的面板（房间账号数据）。
    pub(crate) room_panel_out: watch::Sender<Option<(OwnedRoomId, RoomPanel)>>,
    /// 外部 Matrix 账号的槽位（ADR-0051）：不跟着家里的连接走，直连 homeserver，用登录时存下的状态恢复。
    pub(crate) external: bool,
}

/// 还没登录时的加密状态：只知道本机存没存着恢复密钥。
fn initial_encryption(storage: &ChatStorage) -> Result<EncryptionStatus, CoreError> {
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
    Ok(encryption)
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
            confirmations,
            method,
            app_lock,
            lock_screen,
            session_out,
            room_panel_out,
            external,
        } = channels;
        let storage = ChatStorage::open(data_dir)?;
        let encryption = initial_encryption(&storage)?;
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
            open: Mutex::default(),
            opening: tokio::sync::Mutex::new(()),
            side_chat_support: Arc::default(),
            answers: watch::Sender::default(),
            confirmations: reuse(
                confirmations,
                ConfirmationsViewModel {
                    loading: true,
                    ..Default::default()
                },
            ),
            room_cards: watch::Sender::new(Arc::new(Vec::new())),
            retry_confirmations: Arc::new(tokio::sync::Notify::new()),
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
            app_lock,
            lock_screen,
            draft_writes,
            previews: previews::Previews::new(),
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
        runtime.spawn(pending::follow_cards(
            Arc::downgrade(&chat),
            home.subscribe(),
        ));
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
            action @ (Action::AnswerConfirmation { .. }
            | Action::StopConfirmationWindow { .. }
            | Action::RetryConfirmations) => self.confirmation_action(action),
            Action::SetChatLayout { target, choice } => self.set_chat_layout(target, choice),
            _ => Err(invalid(&l10n!(CATALOG, language(), "chat-invalid-action"))),
        }
    }

    /// 按时间线（主对话或旁聊，ADR-0093 §5）分发的操作：实现和主对话共用，按 [`TimelineKey`] 分开记。
    fn timeline_action(self: &Arc<Self>, action: TimelineAction) -> Result<(), CoreError> {
        match action {
            TimelineAction::SendText {
                target,
                body,
                reply_to,
            } => self.send_text(&TimelineKey::parse(&target)?, &body, reply_to.as_deref()),
            TimelineAction::SendImage {
                target,
                image,
                reply_to,
            } => self.send_image(&TimelineKey::parse(&target)?, image, reply_to.as_deref()),
            TimelineAction::ForwardMessage {
                target,
                item_id,
                to_room_id,
            } => self.forward_message(&TimelineKey::parse(&target)?, &item_id, &to_room_id),
            TimelineAction::RedactMessage { target, item_id } => {
                self.redact_message(&TimelineKey::parse(&target)?, &item_id)
            }
            TimelineAction::RetrySend { target, item_id } => {
                let key = TimelineKey::parse(&target)?;
                let room = self.opened(&key)?;
                let sent = self.sent.clone();
                self.spawn_op_with_success(
                    "chat-op-resend-failed",
                    async move { room.retry_send(&item_id).await },
                    move || onboarding::record_sent(&sent, key.room_id.to_string()),
                );
                Ok(())
            }
            TimelineAction::CancelSend { target, item_id } => {
                let room = self.opened(&TimelineKey::parse(&target)?)?;
                self.spawn_op("chat-op-cancel-send-failed", async move {
                    room.cancel_send(&item_id).await
                });
                Ok(())
            }
            TimelineAction::PaginateBackwards { target } => {
                let room = self.opened(&TimelineKey::parse(&target)?)?;
                self.spawn_op("chat-op-paginate-failed", async move {
                    room.paginate_backwards(PAGE).await.map(drop)
                });
                Ok(())
            }
            TimelineAction::SetTimelineScroll { target, scroll } => {
                self.set_timeline_scroll(&TimelineKey::parse(&target)?, scroll)
            }
            TimelineAction::MarkRead { target } => {
                self.mark_room_read(&TimelineKey::parse(&target)?)
            }
            TimelineAction::SetDraft {
                target,
                body,
                reply_to,
            } => self.set_draft(&TimelineKey::parse(&target)?, &body, reply_to.as_deref()),
            TimelineAction::DismissLinkPreview { target } => {
                self.dismiss_preview(TimelineKey::parse(&target)?);
                Ok(())
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

    pub(crate) fn begin_timeline(
        self: &Arc<Self>,
        target: &TimelineTarget,
    ) -> Result<target::TimelineOpening, CoreError> {
        let key = TimelineKey::parse(target)?;
        let mut open = lock(&self.open);
        let id = open.begin(&key);
        self.room_panel_out.send_if_modified(|current| {
            if current
                .as_ref()
                .is_some_and(|(room, _)| room == &key.room_id)
            {
                return false;
            }
            *current = None;
            true
        });
        Ok(target::TimelineOpening {
            key,
            id,
            chat: Arc::downgrade(self),
        })
    }

    /// 订阅一条时间线（主对话或旁聊）。打开另一个房间会关掉前一个房间的订阅；同一个房间里主对话和一个旁聊
    /// 可以同时开着。
    ///
    /// `room_view` 是 tabs 合成的房间面板视图（面板页、上次看的那页、入口有没有被盖住），每批时间线增量
    /// 都带上当前值；变了也发一批（增量为空）。
    pub(crate) async fn open_timeline(
        self: &Arc<Self>,
        opening: target::TimelineOpening,
        room_view: watch::Receiver<Option<Arc<crate::pages::RoomPanelView>>>,
    ) -> Result<Arc<TimelineSubscription>, CoreError> {
        let key = opening.key.clone();
        // Matrix 的 opened 订阅是单槽；旧请求的副作用必须先结束，才能让新房间接管。
        let _opening = self.opening.lock().await;
        if !lock(&self.open).contains_request(opening.id) {
            return Err(target::not_subscribed(&key));
        }
        let session = self.current()?;
        let room_id = key.room_id.clone();
        if *self.bots.foreground.borrow() {
            self.bots.refresh_all();
        }
        let room = Arc::new(
            open_chat_room(&session, &key)
                .await
                .map_err(|e| core_error(&e))?,
        );
        let menus = Arc::new(Mutex::new(menu::Menus::default()));
        let panel = Self::read_room_panel(&session, &room_id).await;
        if !opening.publish_panel(panel) {
            return Err(target::not_subscribed(&key));
        }
        let mut open = lock(&self.open);
        if !open.contains_request(opening.id) {
            return Err(target::not_subscribed(&key));
        }
        // 进房间时重新核对一次支持声明：Bot 可能是刚连上才写的（ADR-0093 §1）。
        let support = self.side_chat_support.clone();
        support.refresh(&self.runtime, &session, &room_id);
        // 链接的类别要认插件页，插件列表还一次没取过时先取一次（取过或正在取就不重复，免得打乱别处的节奏）。
        let plugins = self.plugins.view();
        if plugins.installed.is_none() && !plugins.refreshing {
            self.plugins.refresh_installed();
        }
        let subscription = TimelineSubscription::start(
            &self.runtime,
            TimelineSource {
                room: room.clone(),
                room_id: room_id.clone(),
                confirmations: self.room_cards.subscribe(),
                plugins: self.plugins.clone(),
                previews: self.previews.clone(),
                menus: menus.clone(),
                room_view,
                chat: self.view.subscribe(),
                support,
            },
            opening,
        );
        open.insert(target::OpenTimeline {
            key,
            room,
            menus,
            subscription: Arc::downgrade(&subscription),
        });
        Ok(subscription)
    }

    /// 读不了时按没有面板开：房间照常打开，主人编辑面板时写不进去，那里会报原因。
    async fn read_room_panel(session: &Arc<MatrixSession>, room_id: &RoomId) -> RoomPanel {
        match session.room_panel(room_id).await {
            Ok(content) => content
                .and_then(|content| serde_json::from_value(content.0.into()).ok())
                .unwrap_or_default(),
            Err(error) => {
                tracing::warn!("could not read the room panel of {room_id}: {error}");
                RoomPanel::default()
            }
        }
    }

    /// 壳报告列表滚到了哪儿：交给这条时间线现在的订阅。
    fn set_timeline_scroll(
        &self,
        key: &TimelineKey,
        scroll: TimelineScroll,
    ) -> Result<(), CoreError> {
        let subscription = lock(&self.open)
            .get(key)
            .and_then(|open| open.subscription.upgrade());
        match subscription {
            Some(subscription) => {
                subscription.set_scroll(scroll);
                Ok(())
            }
            None => Err(target::not_subscribed(key)),
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

    fn opened(&self, key: &TimelineKey) -> Result<Arc<ChatRoom>, CoreError> {
        lock(&self.open)
            .get(key)
            .map(|open| open.room.clone())
            .ok_or_else(|| target::not_subscribed(key))
    }

    /// 已打开的房间直接用，否则打开它。
    ///
    /// 打开会把 Sliding Sync 的房间订阅换成这个房间（同一时间只订阅一个，见 `MatrixSession::open_room`），正在看的
    /// 房间就只剩房间列表带来的最新一条，时间线会断。现在界面只在打开着的房间里发消息、标为已读，走不到这一步；
    /// 以后在列表上做“标为已读”这类不进房间的操作时，要改成不动订阅的写法（比如直接对 SDK 的 `Room` 发回执）。
    async fn room(&self, key: &TimelineKey) -> Result<Arc<ChatRoom>, Error> {
        if let Some(open) = lock(&self.open).get(key) {
            return Ok(open.room.clone());
        }
        let session = self.current().map_err(|e| Error::NotReady(e.to_string()))?;
        Ok(Arc::new(open_chat_room(&session, key).await?))
    }
}

impl ChatLink {
    async fn forget_current(&self) {
        self.bots.set_session(None);
        self.session_out.send_replace(None);
        {
            let mut open = lock(&self.open);
            open.clear();
            self.room_panel_out.send_replace(None);
        }
        *lock(&self.verification) = None;
        self.side_chat_support.clear();
        self.room_order.clear();
        self.sent.send_replace(Vec::new());
        self.home.set_catching_up(false);
        self.view.send_modify(|view| {
            recovery::forgot_session(&mut view.encryption);
            // 确认卡片跟家里的连接走，不和 Matrix 会话绑定：这里不清。
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
            catching_up,
        ];
        tasks.extend(self.verification_tasks(session));
        tasks
    }
}

/// 打开主对话或旁聊的时间线（ADR-0093 §4）。
async fn open_chat_room(session: &MatrixSession, key: &TimelineKey) -> Result<ChatRoom, Error> {
    match key.thread() {
        Some(root) => session.open_thread(&key.room_id, root).await,
        None => session.open_main(&key.room_id).await,
    }
}

/// 核心账号：和登录的账号 `own_user` 在同一台服务器上。通知房间（核心发插件通知、邮件的私聊）以它认，
/// 别人发的同样结构的内容是普通消息。
pub(crate) fn core_account(own_user: &UserId) -> OwnedUserId {
    format!("@{CORE_LOCALPART}:{}", own_user.server_name())
        .parse()
        .expect("the account's server name with the fixed core localpart is still a valid user ID")
}
