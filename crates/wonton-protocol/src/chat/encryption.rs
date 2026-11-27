use serde::{Deserialize, Serialize};

use crate::{SecretString, VerificationFlow};

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
