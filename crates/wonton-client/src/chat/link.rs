//! `ChatLink` 本体：字段、活任务、换服务器时的退役和 Drop，以及还没登录时的加密状态。
//!
//! 字段都在 `chat` 的子模块之间共用：别的文件要读写哪个，就在这儿标 `pub(super)`。

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{Arc, Mutex};

use tokio::runtime::Handle;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use wonton_matrix::MatrixSession;
use wonton_protocol::{
    ChatViewModel, ConfirmationView, ConfirmationsViewModel, CoreError, DeviceVerification,
    EncryptionStatus, PushMethod, RecoveryNextStep, RecoveryStatus,
};

#[cfg(all(test, feature = "ui-fake"))]
use super::TimelineKey;
use super::avatar_files;
use super::bots;
use super::confirmation::Answers;
use super::connect;
use super::draft::DraftWrite;
use super::installs;
use super::layout::Layouts;
use super::onboarding;
use super::previews;
use super::recovery;
use super::side_chat;
use super::target;
use super::util::lock;
use super::verification;
use super::{CardSuccess, ChatStorage};
use crate::home::HomeLink;

pub(crate) struct ChatLink {
    pub(super) runtime: Handle,
    pub(super) home: Arc<HomeLink>,
    /// 本作用域的槽位号（`ServerRow::id`）：后台通知的 `server_id` 填它，壳点开通知按它切服务器；
    /// 不是 home 里的 endpoint ID，那是配对键，槽位列表里没有。
    pub(super) slot: String,
    pub(super) storage: ChatStorage,
    pub(super) view: watch::Sender<ChatViewModel>,
    pub(super) active: tokio::sync::Mutex<Option<Active>>,
    pub(super) session: Mutex<Option<Arc<MatrixSession>>>,
    pub(super) session_out: watch::Sender<Option<Arc<MatrixSession>>>,
    /// 每份订阅独立的滚动位置、菜单；同目标共用 Matrix 时间线。
    pub(super) open: Mutex<target::OpenTimelines>,
    pub(super) opening: tokio::sync::Mutex<()>,
    /// 各房间的旁聊支持声明（ADR-0093 §1），按信任规则核对过。
    pub(super) side_chat_support: Arc<side_chat::Support>,
    /// 这台设备回答确认卡片的进展，时间线的订阅跟着它更新卡片。
    pub(super) answers: watch::Sender<Answers>,
    /// “等你确认”列表（ADR-0016）：长轮询核心的确认接口得到的视图，壳订阅它。换服务器时给壳接着订阅用，
    /// 所以从通道进来。
    pub(super) confirmations: watch::Sender<ConfirmationsViewModel>,
    /// 会话里能显示的卡片（等回答的，加上这个进程里看着结束的），按 `origin.room_id` 由时间线的出口过滤。
    pub(super) room_cards: watch::Sender<Arc<Vec<ConfirmationView>>>,
    /// `ChatAction::RetryConfirmations` 放行长轮询的退避等待。
    pub(super) retry_confirmations: Arc<tokio::sync::Notify>,
    /// 本会话里亲眼看到从没结束变成成功的插件 ID：插件列表多半还是旧的，补取的逻辑在插件那头。
    pub(super) card_success: CardSuccess,
    /// 正在重置恢复。途中 SDK 会报“未设置”，不能覆盖“处理中”或触发自动开启。
    pub(super) resetting_recovery: AtomicBool,

    /// 最近一次“问 Bot”的序号；慢的旧动作不覆盖新回执，dismiss 也靠它作废旧回执。
    pub(super) ask_seq: AtomicU64,
    /// 主人的默认 Agent 在 Matrix 上的用户 ID，聊天列表里它的房间排第一。
    pub(super) default_bot: Arc<Mutex<Option<String>>>,
    /// 房间列表的基准顺序，显示时把默认 Agent 的房间提到最前。
    pub(super) room_order: Arc<onboarding::RoomOrder>,
    /// 发过消息的房间，引导提示用。
    pub(super) sent: watch::Sender<Vec<String>>,
    /// 还没登录 Matrix 时改的显示名，登录后再设。
    pub(super) display_name: Mutex<Option<String>>,
    /// 聊天列表最上面的安装占位行（ADR-0067）。
    pub(super) installs: Arc<installs::Installs>,
    /// 聊天显示方式的账号数据（ADR-0060），只读给房间和 Bot 资料页合成用。
    pub(super) layouts: Arc<Layouts>,
    pub(super) bots: Arc<bots::Bots>,
    pub(super) plugins: Arc<crate::plugins::PluginsLink>,
    pub(super) avatar_files: avatar_files::AvatarFiles,
    /// 收新消息的方式（ADR-0003）：保持连接时后台同步到的新消息要弹通知。
    pub(super) method: watch::Receiver<PushMethod>,
    /// App 锁（ADR-0102）：锁着时弹出的通知不带内容。
    pub(super) app_lock: crate::app_lock::AppLockLink,
    /// 锁屏上隐藏内容的类别（ADR-0101 §5）：后台通知照它设系统的可见性。
    pub(super) lock_screen: watch::Receiver<crate::lock_screen::Hidden>,
    /// 推送里带来的正文预览（ADR-0118）：打开房间时补进时间线。
    pub(super) push_previews: Arc<crate::push::PushPreviews>,
    /// 排队的草稿写入，见 [`draft::write_drafts`]。
    pub(super) draft_writes: mpsc::UnboundedSender<DraftWrite>,
    /// 链接预览的按网址缓存和点掉的记录（ADR-0065），发送框和时间线共用。
    pub(super) previews: previews::Previews,
    /// 测试用：`stop_typing` 收到的目标，确认发送前真的叫过。
    #[cfg(all(test, feature = "ui-fake"))]
    pub(super) typing_stops: Mutex<Vec<TimelineKey>>,
    /// App 在后台：保持连接下同步到的该通知的消息收集进 `background_notifications`。
    pub(super) background: AtomicBool,
    pub(super) verification: Mutex<Option<verification::Control>>,
    /// “添加设备”页开着、还能自动接受一个新设备的确认请求（ADR-0087）。
    pub(super) handoff_armed: AtomicBool,
    pub(super) relogin: connect::Relogin,
    /// 这次登录用的本地代理：隧道刚通时又进来一次同样的代理，就知道不用再换一次会话。
    pub(super) signing_proxy: Mutex<Option<wonton_protocol::ProxySettings>>,
    pub(super) external: bool,
    /// 已经有人在跟的房间，输入框的 @ 列表跟着成员和 Bot 补发，一个房间只跟一次。
    pub(super) mentions_watched: Mutex<HashSet<String>>,
}

pub(super) struct Active {
    pub(super) tasks: Vec<JoinHandle<()>>,
}

impl Drop for Active {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

impl ChatLink {
    /// 不可逆地撤销本人 Bot 的发号与上写资格，不碰 ViewModel。
    ///
    /// 换服务器时共用同一个 ViewModel 发送端，旧链接这时还没 Drop；交接前先撤销，旧作用域关停最多等一秒，
    /// 不能靠 Drop 兜这个窗口；旧同步生命周期调用也不能再发号或清槽。Drop 同样只退役，不清新作用域的快照。
    pub(crate) fn abandon_own_private_bot(&self) {
        self.bots.retire_own_private_bot();
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
        self.abandon_own_private_bot();
        self.bots.session.send_replace(None);
        self.active.get_mut().take();
    }
}

/// 还没登录时的加密状态：只知道本机存没存着恢复密钥。
pub(super) fn initial_encryption(storage: &ChatStorage) -> Result<EncryptionStatus, CoreError> {
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
