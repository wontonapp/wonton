use serde::{Deserialize, Serialize};

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
