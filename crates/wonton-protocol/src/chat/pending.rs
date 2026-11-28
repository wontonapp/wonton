use serde::{Deserialize, Serialize};

/// 一张等主人回答的确认卡片。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PendingConfirmation {
    /// 卡片 ID。
    pub id: String,
    /// 卡片显示在哪个会话（[`ConfirmationCard::origin`](crate::ConfirmationCard::origin) 的房间）：点提示条进这个房间，
    /// 主人正在这个房间时不提示。卡片没有会话时为 `None`，点了打开“等你确认”列表。
    pub room_id: Option<String>,
    /// 卡片的标题，如“Hermes 想安装 文件搜索”。
    pub title: String,
    /// 谁请主人确认，壳按它画头像。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub requester: Option<crate::ConfirmationRequester>,
    /// 到期时间，Unix 毫秒。
    #[serde(default)]
    #[cfg_attr(feature = "uniffi", uniffi(default = 0))]
    pub expires_at_ms: u64,
    /// Bot 已连续等主人回答至少一分钟；Android 可显示进行中通知。重启后重新观察，不猜任务开始时间。
    #[serde(default)]
    #[cfg_attr(feature = "uniffi", uniffi(default = false))]
    pub ongoing: bool,
}
