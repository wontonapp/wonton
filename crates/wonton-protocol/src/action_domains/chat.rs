use serde::{Deserialize, Serialize};

use crate::{Answer, ChatLayoutChoice, ChatLayoutTarget, SecretString};

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
