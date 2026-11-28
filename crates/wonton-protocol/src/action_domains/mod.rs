mod family;
mod session;
mod settings;

mod file;
mod onboarding;
mod pages;
use serde::{Deserialize, Serialize};

use crate::{
    Answer, BotRef, ChatLayoutChoice, ChatLayoutTarget, DeviceNetwork, MatrixOidcCallback,
    MatrixOidcLogin, MatrixPasswordLogin, RelaySettingsAction, SecretString,
};
mod bot_profile;
mod clients;
mod peers;
mod plugin;
mod push;
pub use bot_profile::BotAction;
pub use clients::ClientsAction;
mod sharing;
pub use family::FamilyAction;
mod friends;
pub use file::FileAction;
pub use friends::FriendsAction;
pub use onboarding::OnboardingAction;
pub use pages::PageAction;
pub use peers::PeerAction;
pub use plugin::PluginAction;
pub use push::PushAction;
pub use session::SessionAction;
pub use settings::SettingsAction;
pub use sharing::SharingAction;

/// 家庭服务器、服务器列表、生命周期和外部 Matrix 登录的操作。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HomeAction {
    /// 不经配对，直接用给定的连接信息连接家里的服务器，并存在本机，之后每次启动自动连接。App 走
    /// [`HomeAction::PairHome`]，连接信息由核心按配对码生成；这个操作给测试用。连接信息不合法时返回
    /// `InvalidInput`；连接的进展和结果体现在 [`HomeViewModel`](crate::HomeViewModel) 里。
    ConnectHome {
        ticket: String,
    },
    /// 扫码配对，成功后保存连接信息；配对码或设备名不合法时返回 `InvalidInput`。
    /// 进展和结果见 [`HomeViewModel::pairing`](crate::HomeViewModel::pairing)。
    PairHome {
        code: SecretString,
        device_name: String,
    },
    /// 收起本次配对失败，返回扫码或粘贴；不取消正在进行的配对、不移除已保存的服务器。
    DismissPairingFailure,
    /// 取消正在进行的配对，回到扫码或粘贴；没有正在进行的配对时返回 `InvalidInput`。本机放弃这一次尝试，
    /// 服务器可能已经按交出的码登记了这台设备：同一个码再配对，服务端幂等地回成功，不会卡在“码已用过”。
    CancelPairing,
    /// “添加设备”：经隧道要一个一次性配对码给新设备扫（ADR-0009），页面开着时自动接上新设备的确认请求（ADR-0087）。
    /// 没配对时返回 `NotReady`；进展和码在 `HomeViewModel::invite`。再发一次换新码，旧码到期前仍能用。
    InviteDevice,
    CloseDeviceInvite,
    /// 添加一台服务器：在一个新位置上走配对，当前那台的连接先断开、数据留着。配对成功后新的那台成为当前这台；
    /// 配对前可以 [`HomeAction::CancelAddServer`] 回去（ADR-0042）。
    AddServer,
    /// 放弃添加，回到原来那台服务器。没在添加时返回 `InvalidInput`。
    CancelAddServer,
    /// 换到另一台配过的服务器：断开当前这台（进行中的传输暂停，换回来接着传），连上那台。`id` 是
    /// [`ServerRow::id`](crate::ServerRow::id)。
    SwitchServer {
        id: String,
    },
    /// 给配过的服务器改名，只存在这台手机上。名字去掉首尾空白后不能为空。
    RenameServer {
        id: String,
        name: String,
    },
    /// 从这台手机上移除一台服务器：删掉它在手机上的全部数据，不吊销这台设备（吊销在服务器那边）。移除的是当前这台时
    /// 换到剩下的第一台；一台都不剩时回到配对页。
    RemoveServer {
        id: String,
    },
    /// 开始添加外部 Matrix 账号，None 使用 matrix.org；方式和进展由服务器列表的 matrix_login 给出。
    StartMatrixLogin {
        homeserver: Option<String>,
    },
    SubmitMatrixPassword {
        login: MatrixPasswordLogin,
    },
    StartMatrixOidc {
        login: MatrixOidcLogin,
    },
    CompleteMatrixOidc {
        callback: MatrixOidcCallback,
    },
    CancelMatrixLogin {
        login_id: String,
    },
    /// App 回到前台。核心检查隧道和本地代理是否还能用，必要时重建；系统挂起 App 期间可能回收了套接字。
    /// Matrix 同步停了的话也在这时重启。
    AppForegrounded,
    /// App 进了后台。核心停止探测和自动重连，直到 [`HomeAction::AppForegrounded`]；连接不主动断，后台音乐和正在传的文件
    /// 还在用它。
    AppBackgrounded,
    /// 系统报告网络变了。壳每次收到系统回调都发，不做判断：核心让 iroh 重新检查路径，并立刻重试（ADR-0032）。
    NetworkChanged {
        network: DeviceNetwork,
    },
    /// 主人点了“重试”：清零退避，立刻试一次。
    RetryConnection,
    /// 中继表设置页（ADR-0048）上的操作：取表、加、改、删、挪档、设置 n0 兜底、保存、丢弃。
    /// 进展和结果在 [`HomeViewModel::relays`](crate::HomeViewModel::relays)。
    RelaySettings {
        action: RelaySettingsAction,
    },
}

/// Matrix 聊天、加密恢复、确认卡片和聊天布局的操作。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ChatAction {
    /// 接受房间邀请。
    JoinRoom {
        room_id: String,
    },
    /// 拒绝邀请，或者离开房间。
    LeaveRoom {
        room_id: String,
    },
    /// 和另一个用户（比如 Bot）发起私聊；已经有和他的私聊时什么都不做，对方不存在时失败。`user_id` 是完整的 ID，
    /// 主人输入的先用客户端核心的 `resolve_user_id` 补全。
    StartDirectChat {
        user_id: String,
    },
    /// 把一句话发给默认 Bot（Siri、快捷指令的“问 Bot”）：哪个 Bot、发到哪个房间由核心解析，结果写进
    /// [`ChatViewModel::ask`](crate::ChatViewModel::ask)。`text` 为空时只解析目标、打开那个私聊，不发消息。
    ///
    /// `room_id` 是壳当前所在的房间，原样带上：只有它是呼叫者可用 Bot 的私聊，或者本人 Bot 的主房间时才用它。
    /// `request_id` 由壳生成，用来把回执对上同一次动作；核心不解释它。
    AskAgent {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "uniffi", uniffi(default = None))]
        room_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "uniffi", uniffi(default = None))]
        request_id: Option<u32>,
    },
    /// 点了消息下的选项按钮（[`MessageChoice`](crate::MessageChoice)，ADR-0109）或 AgentRun 的选项：以本人身份给 `item_id`
    /// 那条加 `key` 这个表情（`m.annotation`），不往聊天里发消息。进展在那个选项的 [`ChoiceSend`](crate::ChoiceSend)；
    /// 失败了再发一次就是重发。这组选项不能点（[`ChoicesState`](crate::ChoicesState) 不是 `Open`）或没有这个 `key` 时
    /// 返回 `InvalidInput`。
    AnswerAgentChoice {
        room_id: String,
        item_id: String,
        key: String,
    },
    /// 重置恢复：删掉账号的密钥备份和秘密存储，换新的交叉签名身份，本设备成为新身份下已验证的设备，再开启备份，
    /// 新的恢复密钥放进 `EncryptionStatus::recovery_key`。旧的恢复密钥作废，备份里的旧消息取不回了，其他设备要
    /// 输入新的恢复密钥。主人不用输密码，核心发的登录令牌就能向服务器确认身份。
    ResetRecovery,
    /// 主人已经把恢复密钥另外保存：核心把它从快照和本机清掉，之后不再显示。
    ConfirmRecoveryKeySaved,
    /// 用恢复密钥恢复：取回交叉签名的秘密和密钥备份，本设备随之被验证，能解密历史消息。
    RecoverEncryption {
        recovery_key: SecretString,
    },
    /// 请账号里的另一台设备确认本设备（ADR-0046）：发给自己账号已验证的设备，进展在
    /// `EncryptionStatus::verification_flow`。`EncryptionStatus::can_verify_with_other_device` 为假时前置条件不满足。
    StartDeviceVerification,
    /// 接受另一台设备发来的确认请求，随后两边出表情。
    AcceptDeviceVerification {
        flow_id: String,
    },
    /// 主人看到两台设备上的表情一样。
    ConfirmVerification {
        flow_id: String,
    },
    /// 主人看到两台设备上的表情不一样：取消这次确认，不信任对方。
    DenyVerification {
        flow_id: String,
    },
    /// 取消这次确认，或者拒绝收到的请求。
    CancelDeviceVerification {
        flow_id: String,
    },
    SkipKeyHandoff,
    /// 清掉 `ChatViewModel::problem`。
    DismissChatProblem,
    /// 只确认会话装好没有：没登录返回 `NotReady`，登录了什么都不做、直接成功。给壳和测试在真正 dispatch 前自查
    /// （例如等到登录真的能用再发第一条），不碰房间、不写账号数据。
    CheckReady,
    /// 按卡片上的 `answer`、`choice_id` 回答；结果在 [`ConfirmationView`](crate::ConfirmationView)，已经在发或已答完时不重复发送。
    /// 答案为 `Unknown` 返回 `InvalidInput`；服务器未连接时返回 `NotReady`。
    AnswerConfirmation {
        id: String,
        answer: Answer,
        #[cfg_attr(feature = "uniffi", uniffi(default = None))]
        choice_id: Option<String>,
        /// 装、升级插件的卡片上填的密钥；别的卡片是空表。
        secrets: crate::SecretValues,
        /// 装、升级插件的卡片上填的设置；别的卡片是空表。
        #[serde(default)]
        settings: crate::SettingValues,
        /// 安装或升级时主人选择授权的设备。
        #[serde(default)]
        devices: Vec<String>,
    },
    /// 主人点了卡片上放行时段的“停止”：只拦后面的调用，已经发给插件的不撤回（ADR-0024）。进展和结果在
    /// [`ConfirmationView::window`](crate::ConfirmationView::window)。已经在发时什么都不做，家里的服务器还没连上时返回
    /// `NotReady`。
    StopConfirmationWindow {
        id: String,
    },
    /// 重新取“等你确认”列表（[`ConfirmationsViewModel::problem`](crate::ConfirmationsViewModel::problem) 之后）。
    RetryConfirmations,
    /// 把这一份的选择换成 `choice`：为空的项回到默认值。
    SetChatLayout {
        target: ChatLayoutTarget,
        choice: ChatLayoutChoice,
    },
}

/// Bot 联系人操作。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BotContactAction {
    /// 打开某个 Bot 的联系人或下拉刷新（`GET`
    /// [`bot_contacts_path`](crate::bot_contacts_path)），之后的结果都是这个 Bot 的。
    RefreshBotContacts { bot: BotRef },
    /// 请另一个 Bot（本机的，或配对过的服务器上的）做这个 Bot 的联系人，等它的归属人同意。
    RequestBotContact { bot: BotRef, user_id: String },
    /// 请配对过的服务器上的一个人做这个 Bot 的联系人：Bot 给他发带标记的私聊邀请，他接受才算数，之前一直是“等对方接受”。
    RequestPersonContact { bot: BotRef, user_id: String },
    /// 同意对方发来的请求（[`ContactState::Incoming`](crate::ContactState::Incoming)）。
    ApproveBotContact { bot: BotRef, user_id: String },
    /// 拒绝对方发来的请求。
    RejectBotContact { bot: BotRef, user_id: String },
    /// 删掉联系人、取消自己发出的请求，或清掉“对方拒绝了”“没有这个人”的行；同时通知对方。
    RemoveBotContact { bot: BotRef, user_id: String },
}
