use serde::{Deserialize, Serialize};

/// 核心对外返回的错误。每个变体对应一种原因，壳据此决定怎么提示，不把不同原因混成一种。
///
/// 后台进行的网络操作不走这里：连接家里服务器的进展和失败原因体现在 `HomeViewModel` 里，聊天的体现在
/// `ChatViewModel` 和时间线里。直接返回结果的调用（如取媒体）按 AGENTS.md 的分类返回下面的原因。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Error), uniffi::export(Display))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CoreError {
    /// 操作的参数不满足要求，或引用了不存在的对象。
    #[error("{reason}")]
    InvalidInput { reason: String },
    /// 读写本机存储失败（设备密钥、连接信息、聊天会话、外观设置）。
    #[error("本机存储读写失败：{reason}")]
    Storage { reason: String },
    /// 前置条件不满足：聊天还没登录、房间还没同步到本机等。
    #[error("尚未就绪：{reason}")]
    NotReady { reason: String },
    /// 连不上家里的服务器或它上面的服务：隧道没连上、网络断开或请求超时。
    #[error("连不上服务器：{reason}")]
    TransportUnavailable { reason: String },
    /// 服务器拒绝了身份或权限：设备已被吊销、无权访问。
    #[error("服务器拒绝了请求：{reason}")]
    Unauthorized { reason: String },
    /// 服务器上的服务出错。
    #[error("服务器出错：{reason}")]
    Upstream { reason: String },
}
