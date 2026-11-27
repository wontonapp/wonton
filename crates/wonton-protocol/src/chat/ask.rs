use serde::{Deserialize, Serialize};

/// 一次“问 Bot”动作的回执。同一次动作只有一份，按 `request_id` 归属；壳只认自己发起的那一个。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct AskReceipt {
    /// 壳发起动作时给的 ID；旧壳没给时为空。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub request_id: Option<u32>,
    /// 这次动作解析出的房间：`Pending` 时还没解析出来为空；`Sent` 时是实际发送的房间；`Failed` 时是试过、
    /// 被拒的房间（没有目标时为空）。壳只按它打开私聊，不按对方用户 ID 扫房间。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub room_id: Option<String>,
    pub state: AskState,
    /// 失败的原因和详情；`Pending`、`Sent` 时为空。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub problem: Option<AskProblem>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum AskState {
    /// 核心已经接下这次动作，正在找房间或发送。
    Pending,
    /// 已经交给 Matrix 服务器；只说明服务器收下了，不说明 Bot 读过。
    Sent,
    Failed,
    #[serde(other)]
    Other,
}

/// 问 Bot 失败的原因。类别由核心给，主句由壳按类别写；`detail` 是核心按 App 语言写的补充。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct AskProblem {
    pub reason: AskFailure,
    /// 原始原因（含服务器给的说明），给默认收起的“详情”用；没有时为空，主句由壳按类别写。
    pub detail: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum AskFailure {
    /// 当前人没有可用的 Bot：没有本人 Bot，默认 Bot 也不可用。
    NoBot,
    /// 有 Bot，但它还没就绪（身份待核实、主房间还没同步、私人 Bot 还在建）。
    BotNotReady,
    /// Bot 的身份或主房间记录坏了。
    BotProblem,
    /// 壳给的房间不是这个人的 Bot 私聊，也不是本人 Bot 的主房间。
    NotInRoom,
    /// 壳给的房间是只读的通知私聊。
    ReadOnlyRoom,
    /// 网关拒绝：对方还不是这个 Bot 的联系人。
    ContactRequired,
    /// 连不上家里的服务器。
    TransportUnavailable,
    /// 服务器不让发（登录还有效）。
    Forbidden,
    /// 服务器或会话还没就绪。
    NotReady,
    /// 服务器或 Matrix SDK 出错。
    Upstream,
    #[serde(other)]
    Other,
}
