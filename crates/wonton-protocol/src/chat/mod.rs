//! 聊天：Matrix 账号、同步、端到端加密和房间列表（ADR-0003、ADR-0005）。
//!
//! 客户端核心经隧道把家里的 Tuwunel 当成标准的 Matrix 服务器用（ADR-0015）。这里的 ViewModel 是整份快照：
//! 来不及取走的中间状态会被合并，壳拿到后直接渲染。打开的房间的消息走增量，见 `timeline` 模块。

use serde::{Deserialize, Serialize};

use crate::{ChatLayout, SecretString, VerificationFlow};

mod ask;
mod contacts;
mod encryption;
mod install;
mod media;
mod own_bot;
mod pending;
mod rooms;
mod session;
#[cfg(test)]
mod tests;
mod view;

/// 本设备的 Matrix 账号。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ChatAccount {
    /// 还没有登录：家里的服务器还没连上，或者还没拿到登录凭据。
    SignedOut,
    /// 正在登录，或者正在恢复上次保存的会话。
    SigningIn,
    /// 已登录。会话存在本机，重启后直接恢复，不用重新登录。
    SignedIn {
        /// Matrix 用户 ID，如 `@owner:home.example`。
        user_id: String,
        /// 本设备在服务器上的设备 ID。
        device_id: String,
    },
    /// 上一次登录因为暂时的原因没成功（连不上、Matrix 还没就绪），核心正按退避自动重试，不用用户操作。
    /// `reason`、`message` 是上一次失败的原因。
    Reconnecting {
        reason: ChatFailure,
        message: String,
    },
    /// 登录失败、不会自动重试（App 回到前台时再试一次），或者会话已失效。`Unauthorized` 表示这台设备被吊销了，
    /// 要主人重新配对。
    Failed {
        reason: ChatFailure,
        message: String,
    },
}

/// 同步状态。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ChatSync {
    /// 没在同步：还没登录，或者 App 在后台。
    Idle,
    /// 正在同步，房间列表还没收全。
    Starting,
    /// 正常同步中。
    Running,
    /// 同步因错误停止。核心会在网络恢复或回到前台时重试；`Unauthorized` 不会重试。
    Failed {
        reason: ChatFailure,
        message: String,
    },
}

/// 聊天相关失败的原因，壳据此决定怎么提示（AGENTS.md：错误必须区分原因）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum ChatFailure {
    /// 连不上 Matrix 服务器：隧道没连上、网络断开或请求超时。
    TransportUnavailable,
    /// 登录失效：登录凭据无效或过期、设备已被吊销、账号已停用。要重新登录或配对。
    Unauthorized,
    /// 登录有效，但服务器不让做这件事：没有权限、对方屏蔽了自己。不是要重新登录。
    Forbidden,
    /// 对方服务器还没和这台配对（ADR-0050）：跨服务器的私聊要先在“其他服务器”里配对。
    ServerNotPaired,
    /// 对方服务器的公网访问关着（ADR-0090 第 2 节）：经公网才联系得到的对方暂时收不到，要等对方重新打开。
    PeerPublicAccessOff,
    /// 前置条件不满足：服务器缺少必需的能力，或者要操作的房间还没同步到本机。
    NotReady,
    /// 服务器或 Matrix SDK 出错。
    Upstream,
    /// 输入非法。
    InvalidInput,
    /// 比这个壳新的类别。
    #[serde(other)]
    Other,
}

/// 后台操作失败的说明。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ChatProblem {
    pub reason: ChatFailure,
    /// 给人看的说明，包含失败的操作和原因。
    pub message: String,
}

/// 端到端加密的状态。设备密钥、交叉签名密钥和密钥备份都存在本机的存储里，重启后不丢。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct EncryptionStatus {
    pub verification: DeviceVerification,
    pub recovery: RecoveryStatus,
    /// 刚设置好恢复时生成的恢复密钥，主人用它在别的设备上取回加密消息的密钥。只在主人确认已经另外保存
    /// （[`ChatAction::ConfirmRecoveryKeySaved`](crate::ChatAction::ConfirmRecoveryKeySaved)）之前给出，确认后核心
    /// 把它从快照和本机清掉；其余时候为空。
    pub recovery_key: Option<SecretString>,
    /// 聊天列表需要主人处理的备份提醒；文字和跳转由原生壳呈现。
    pub backup_reminder: Option<BackupReminder>,
    /// 本设备没验证，账号里有别的已验证设备：可以请它确认（[`ChatAction::StartDeviceVerification`](crate::ChatAction::StartDeviceVerification)），和输入恢复密钥并列。
    ///
    /// [`ChatAction::StartDeviceVerification`](crate::ChatAction::StartDeviceVerification)：请求另一个已验证设备确认。
    pub can_verify_with_other_device: bool,
    /// 备份页显不显示“重置备份”（[`ChatAction::ResetRecovery`](crate::ChatAction::ResetRecovery)）：只在缺密钥、备份残缺时显示——恢复残缺，或者账号
    /// 还没有备份而本设备又没验证。备份完好不显示，新密钥还没确认保存也不显示（还没抄下来就作废）。
    #[serde(default)]
    pub can_reset: bool,
    /// 备份页的下一步给主人看什么。核心按恢复状态算好（ADR-0076），壳只按类别画，不再自己判断。
    #[serde(default)]
    pub recovery_next_step: RecoveryNextStep,
    /// 正在进行或刚结束的设备确认（ADR-0046），本设备发起的和收到的都在这里。
    pub verification_flow: Option<VerificationFlow>,
    /// 加设备时的密钥交接（ADR-0087）：本设备是刚加进来、在等密钥的新设备，或者是开着“添加设备”页、替新设备交出
    /// 密钥的旧设备。没在交接时为空。
    #[serde(default)]
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub key_handoff: Option<crate::KeyHandoff>,
}

/// 备份页的下一步给主人看什么（ADR-0076）。核心按恢复状态算好，壳只按类别画，不自己判断。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum RecoveryNextStep {
    /// 输入恢复密钥：账号设过备份、本设备缺密钥，账号里也没有能确认本设备的其他已验证设备。
    EnterKey,
    /// 两条路并列：请账号里别的已验证设备确认本设备，或者输入恢复密钥，主人选一条。
    ChooseDeviceOrKey,
    /// 走设备确认这条路：流程已经在进行时这一屏就是流程，还没开始时壳画“请另一台设备确认”的入口按钮
    /// （账号还没有备份，输恢复密钥没用，确认是拿到密钥的唯一路）。
    ShowFlow,
    /// 恢复正在进行，壳画进度。
    Progress,
    /// 刚加进来的设备在等另一台设备交出密钥（ADR-0087）：壳画交接页，不提恢复密钥。
    WaitForOtherDevice,
    /// 没有要主人做的：备份完好，或者状态还没同步到。
    #[default]
    None,
    /// 更新的核心给出的类别：壳不画下一步。
    #[serde(other)]
    Other,
}

/// 聊天备份需要主人处理的事项。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum BackupReminder {
    SaveRecoveryKey,
    EnterRecoveryKey,
    /// 密钥交接还没完成，主人先进了聊天列表（ADR-0087）：提醒只说“在另一台设备上继续”，点了回到交接页。
    ContinueOnOtherDevice,
    #[serde(other)]
    Other,
}

/// 本设备有没有被自己账号的交叉签名身份验证过。验证过的设备能解密历史消息，别人也会信任它。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum DeviceVerification {
    /// 还不知道，比如还没登录或者还在同步。
    Unknown,
    Verified,
    /// 账号已经有交叉签名身份，但本设备还没被验证：用恢复密钥恢复即可。
    Unverified,
}

/// 恢复（密钥备份加秘密存储）的状态。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum RecoveryStatus {
    /// 还不知道，比如还没登录或者还在同步。
    Unknown,
    /// 账号还没设置恢复：已验证的设备在聊天会话就绪后自动开启。
    Disabled,
    /// 正在设置或恢复。
    InProgress,
    /// 已设置，本设备也拿到了全部秘密。
    Enabled,
    /// 账号设置过恢复，但本设备还没拿到秘密：用
    /// [`ChatAction::RecoverEncryption`](crate::ChatAction::RecoverEncryption) 输入恢复密钥；密钥丢了或者备份残缺，
    /// 用 [`ChatAction::ResetRecovery`](crate::ChatAction::ResetRecovery) 重置。
    Incomplete,
}

/// 房间里的 Bot 对方：精确到插件里的 Bot 名，和当前请求人能不能聊。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct RoomBot {
    pub bot: crate::BotRef,
    /// Bot 的 Matrix 用户 ID。
    pub user_id: String,
    /// 插件名，按请求人的语言挑过。
    pub title: String,
    pub access: crate::BotAccess,
}

/// 房间列表里的一行。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct RoomRow {
    /// Matrix 房间 ID，操作里用它引用房间。
    pub room_id: String,
    /// 显示名：房间名，私聊时是对方的名字。
    pub name: String,
    /// 是否是私聊。
    pub is_direct: bool,
    /// 私聊对方的 Matrix 用户 ID，壳按它找和某个用户（如插件的 Bot）的私聊。加入了的私聊是 `m.direct` 里记的
    /// 对方，记了不止一个时为空；收到邀请、还没加入的私聊是邀请人。不是私聊时为空。
    pub direct_user_id: Option<String>,
    /// 私聊背后的插件未就绪；正常运行或不是插件私聊时为空。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub plugin: Option<crate::RoomPlugin>,
    /// 私聊对方是 Bot 时当前人能不能聊（[`crate::BotChatAvailability`]）；不是 Bot 的私聊为空。
    /// 不可用时房间和历史都留着，壳禁用输入并写原因。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub bot_chat: Option<crate::BotChatAvailability>,
    /// 私聊对方是谁、凭什么能用（[`crate::BotAccess`]）。`bot_chat` 只有能不能，这里还带精确的 Bot 身份和原因；
    /// 插件没开给当前人时也靠它画出“为什么用不了”。不是 Bot 的私聊为空。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub bot: Option<RoomBot>,
    /// 是否开了端到端加密。
    pub is_encrypted: bool,
    /// 只读的房间（和核心账号的通知私聊）：没有人读这里的消息，壳不显示输入框，发文字和图片会被拒绝。
    pub is_read_only: bool,
    pub membership: RoomMembership,
    /// 最新一条消息的预览；还没有消息或还没同步到时为空。
    pub preview: Option<RoomPreview>,
    /// 房间里有旁聊支持声明（ADR-0093 §1）：可以开新旁聊，旁聊有结果行。声明还没同步到时是 false。
    #[serde(default)]
    pub side_chats: bool,
    /// 未读消息数。
    pub unread_count: u64,
    /// 打开这个房间时怎么显示（ADR-0060）。
    pub layout: ChatLayout,
    /// 房间里有人在通话（ADR-0096），和 [`TimelineUpdate::call_active`](crate::TimelineUpdate::call_active)
    /// 同一个来源：列表这一行标“通话中”。
    #[serde(default)]
    #[cfg_attr(feature = "uniffi", uniffi(default = false))]
    pub call_active: bool,
    /// 未读里提到我的条数（ADR-0121）：大于 0 时房间行的徽标换成「@」。
    #[serde(default)]
    #[cfg_attr(feature = "uniffi", uniffi(default = 0))]
    pub unread_mentions: u64,
}

/// 本账号在房间里的成员状态。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum RoomMembership {
    Joined,
    /// 收到邀请，还没加入：用 [`ChatAction::JoinRoom`](crate::ChatAction::JoinRoom) 接受。
    Invited,
    /// 已离开或被移出。
    Left,
}

/// 房间最新一条消息的预览。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct RoomPreview {
    /// 发送者的显示名。
    pub sender_name: String,
    /// 是否由本账号发出（包括本账号的其他设备）。
    pub is_own: bool,
    /// 消息的类别和正文，壳按类别写占位的文字。
    pub body: MessageSummary,
    /// 发送时间，Unix 毫秒。
    pub timestamp_ms: u64,
}

/// 一行预览用的消息内容。核心只给类别和原文，不拼出要显示的文字。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MessageSummary {
    /// 正文。别人的消息是 Markdown 摊平后的纯文本，自己的是原文。
    Text {
        text: String,
    },
    /// 只有文件链接，没有正文。
    Files,
    Image,
    /// 本机还没有房间密钥，解不开。
    Undecryptable,
    Redacted,
    /// 旁聊（ADR-0093）：根消息和结果行在聊天列表里的预览。
    SideChat {
        title: String,
        state: crate::SideChatState,
    },
    /// 其他类型的事件，`kind` 是事件类型原文，如 `m.sticker`。
    Other {
        kind: String,
    },
}

/// 要发送的图片。壳从相册或相机取到原图后交给核心，核心负责上传和发送。
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ImageUpload {
    /// 文件名，接收方保存时使用。
    pub filename: String,
    /// MIME 类型，必须是图片类型，如 `image/jpeg`。
    pub mime_type: String,
    /// 原图字节。
    pub data: Vec<u8>,
    /// 像素宽度，接收方据此预留版面；未知时为空。
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// 壳生成的缩略图。加密房间里服务器做不了缩略图，对方预览只能下载原图，走中继时很慢（ADR-0004 实测
    /// 4 MB 要 50 多秒），所以由发送端生成；壳生成不了时为空。
    pub thumbnail: Option<ImageThumbnail>,
}

/// 发送端生成的缩略图，建议长边不超过 800 像素的 JPEG。
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ImageThumbnail {
    /// MIME 类型，如 `image/jpeg`。
    pub mime_type: String,
    pub data: Vec<u8>,
    /// 像素宽度和高度。
    pub width: u32,
    pub height: u32,
}

impl std::fmt::Debug for ImageUpload {
    /// 只打字节数：原图可能有几 MB，不能整个写进日志。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImageUpload")
            .field("filename", &self.filename)
            .field("mime_type", &self.mime_type)
            .field("data", &format_args!("<{} bytes>", self.data.len()))
            .field("width", &self.width)
            .field("height", &self.height)
            .field("thumbnail", &self.thumbnail)
            .finish()
    }
}

impl std::fmt::Debug for ImageThumbnail {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImageThumbnail")
            .field("mime_type", &self.mime_type)
            .field("data", &format_args!("<{} bytes>", self.data.len()))
            .field("width", &self.width)
            .field("height", &self.height)
            .finish()
    }
}

pub use ask::{AskFailure, AskProblem, AskReceipt, AskState};
pub use contacts::ChatContact;
pub use install::{BotInstallRow, BotInstallState};
pub use own_bot::{OwnBotFailure, OwnBotProblem, OwnPrivateBotView};
pub use pending::PendingConfirmation;
pub use view::ChatViewModel;
