use serde::{Deserialize, Serialize};

/// 本人私人 Bot 的身份事实，不表示主房间可发送。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum OwnPrivateBotView {
    NotApplicable,
    Absent,
    Ready {
        bot: crate::BotRef,
        user_id: String,
        home_room: Option<String>,
        room_problem: Option<crate::PrivateBotRoomProblem>,
    },
    #[serde(other)]
    Other,
}

/// 读取本人私人 Bot 身份时的问题，与聊天收发错误分开。
/// 壳按 `category` 复用既有 Problem/ProblemRow 控件，`detail` 进正文；`kind` 不直接显示。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct OwnBotProblem {
    pub category: OwnBotFailure,
    /// 核心给出的机器类别，不用于生成直接显示的文字。
    pub kind: Option<String>,
    pub status: Option<u16>,
    /// 不含服务器原始消息或输入值的本地化详情。
    pub detail: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum OwnBotFailure {
    TransportUnavailable,
    Unauthorized,
    PermissionDenied,
    CapabilityUnknown,
    NotReady,
    RecordInconsistent,
    StorageUnavailable,
    StorageCorrupt,
    Upstream,
    Protocol,
    #[serde(other)]
    Other,
}
