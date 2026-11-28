//! `ChatLink` 的创建和外壳接口：首次快照、回到前台、`ChatAction` 分派，订阅的滚动位置和查找，未读跳转，
//! 以及媒体和不打开房间的发送。

use std::collections::HashSet;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::runtime::Handle;
use tokio::sync::{mpsc, watch};
use wonton_l10n::l10n;
use wonton_protocol::{
    ChatAccount, ChatAction, ChatFailure, ChatSync, ChatViewModel, ConfirmationsViewModel,
    CoreError, EncryptionStatus, MediaHandle, PushStatus, TimelineScroll,
};

use super::avatar_files;
use super::background::NotifyBoundary;
use super::bots;
use super::connect;
use super::core_error;
use super::draft;
use super::follow_home;
use super::installs;
use super::layout::Layouts;
use super::link::initial_encryption;
use super::onboarding;
use super::pending;
use super::previews;
use super::send;
use super::util::{invalid, lock};
use super::{CardSuccess, ChatChannels, ChatLink, ChatStorage, TimelineKey, TimelineSubscription};
use crate::home::HomeLink;
use crate::l10n::{CATALOG, language};
use crate::scope::reuse;

impl ChatLink {
    /// 首次快照：壳复用的通道直接沿用，空快照按这里初始化。
    fn initial_view(
        view: Option<watch::Sender<ChatViewModel>>,
        encryption: EncryptionStatus,
    ) -> watch::Sender<ChatViewModel> {
        reuse(
            view,
            ChatViewModel {
                ask: None,
                chat_layout_default: Default::default(),
                account: ChatAccount::SignedOut,
                sync: ChatSync::Idle,
                encryption,
                installing_bots: Vec::new(),
                rooms: Vec::new(),
                shared_contacts: Vec::new(),
                problem: None,
                push: PushStatus::NoEndpoint,
                pending_confirmations: Vec::new(),
                settled_confirmations: Vec::new(),
                background_notifications: Vec::new(),
                bots: Vec::new(),
                own_private_bot: None,
                own_private_bot_problem: None,
            },
        )
    }

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
            external,
            push_previews,
        } = channels;
        let storage = ChatStorage::open(data_dir)?;
        let view = Self::initial_view(view, initial_encryption(&storage)?);
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
            ask_seq: AtomicU64::default(),
            default_bot: Arc::new(Mutex::new(None)),
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
            #[cfg(all(test, feature = "ui-fake"))]
            typing_stops: Mutex::default(),
            background: AtomicBool::new(false),
            notify_boundary: NotifyBoundary::default(),
            verification: Mutex::new(None),
            handoff_armed: AtomicBool::new(false),
            relogin: connect::Relogin::default(),
            signing_proxy: Mutex::default(),
            external,
            mentions_watched: Mutex::new(HashSet::new()),
            push_previews,
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
    pub(crate) fn dispatch(self: &Arc<Self>, action: ChatAction) -> Result<(), CoreError> {
        match action {
            ChatAction::DismissChatProblem => {
                self.invalidate_ask();
                self.view.send_modify(|view| view.problem = None);
                Ok(())
            }
            // 只取一次会话：没登录就是 NotReady，有就什么都不做。
            ChatAction::CheckReady => self.current().map(drop),
            ChatAction::JoinRoom { room_id } => {
                let (session, room_id) = self.target(&room_id)?;
                self.spawn_op("chat-op-join-failed", async move {
                    session.join_room(&room_id).await
                });
                Ok(())
            }
            ChatAction::LeaveRoom { room_id } => {
                let (session, room_id) = self.target(&room_id)?;
                self.spawn_op("chat-op-leave-failed", async move {
                    session.leave_room(&room_id).await
                });
                Ok(())
            }
            ChatAction::StartDirectChat { user_id } => self.start_direct_chat(&user_id),
            ChatAction::AnswerAgentChoice {
                room_id,
                item_id,
                key,
            } => self.answer_agent_choice(&room_id, &item_id, &key),
            ChatAction::ResetRecovery => self.reset_recovery(),
            ChatAction::ConfirmRecoveryKeySaved => self.confirm_recovery_key_saved(),
            ChatAction::RecoverEncryption { recovery_key } => self.recover(&recovery_key),
            action @ (ChatAction::StartDeviceVerification
            | ChatAction::AcceptDeviceVerification { .. }
            | ChatAction::ConfirmVerification { .. }
            | ChatAction::DenyVerification { .. }
            | ChatAction::CancelDeviceVerification { .. }) => self.verification_action(action),
            action @ (ChatAction::AnswerConfirmation { .. }
            | ChatAction::StopConfirmationWindow { .. }
            | ChatAction::RetryConfirmations) => self.confirmation_action(action),
            ChatAction::SetChatLayout { target, choice } => self.set_chat_layout(target, choice),
            ChatAction::AskAgent { .. } | ChatAction::SkipKeyHandoff => {
                Err(invalid(&l10n!(CATALOG, language(), "chat-invalid-action")))
            }
        }
    }

    /// 取好读位置和未读数再让跳转状态机去分页（ADR-0121），别在分派里等网络。
    pub(super) fn jump_to_unread(&self, key: &TimelineKey, mention: bool) -> Result<(), CoreError> {
        let subscription = self.subscription(key)?;
        let room = self.opened(key)?;
        self.runtime.spawn(async move {
            let position = subscription
                .unread_read_item()
                .or(room.read_position().await.map(|id| id.to_string()));
            let unread = u32::try_from(room.unread_messages().await).unwrap_or(u32::MAX);
            if mention {
                subscription.jump_to_next_mention(position, unread);
            } else {
                subscription.jump_to_first_unread(position, unread);
            }
        });
        Ok(())
    }

    /// 壳报告列表滚到了哪儿：交给这条时间线现在的订阅。
    pub(super) fn set_timeline_scroll(
        &self,
        key: &TimelineKey,
        scroll: TimelineScroll,
    ) -> Result<(), CoreError> {
        self.subscription(key)?.set_scroll(scroll);
        Ok(())
    }

    pub(super) fn subscription(
        &self,
        key: &TimelineKey,
    ) -> Result<Arc<TimelineSubscription>, CoreError> {
        lock(&self.open)
            .get(key)
            .and_then(|open| open.subscription.upgrade())
            .ok_or_else(|| self.not_subscribed(key))
    }

    /// 测试用：`stop_typing` 收到的目标。
    #[cfg(all(test, feature = "ui-fake"))]
    pub(crate) fn typing_stops_for_test(&self) -> Vec<TimelineKey> {
        lock(&self.typing_stops).clone()
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

    /// 等登录开始（最多 `wait`）：App 刚打开时壳还没开始连，账号是 `SignedOut`；等一小会儿，别当场判断没登录。
    /// 已经登录、正在登录/重连就立刻返回真。
    pub(crate) async fn sign_in_started_within(&self, wait: Duration) -> bool {
        send::sign_in_started_within(self.subscribe(), wait).await
    }

    /// 等账号登录好（最多 `wait`）。登录失败的间隙会自己重连，看到失败后再等 `failed_grace`，等不到就返回。
    pub(crate) async fn wait_signed_in(&self, wait: Duration, failed_grace: Duration) {
        send::wait_signed_in(self.subscribe(), wait, failed_grace).await
    }
}

/// 发消息等多久：核心回 502 时 matrix-sdk 自己要十几分钟才放弃，期限盖住它；被放弃的那条还会留在 SDK 的发送
/// 队列里按自己的退避重试，重发的消息排在后面，30 秒撑不住慢机器，给满一分钟。
const SEND_TIMEOUT: Duration = Duration::from_secs(60);
