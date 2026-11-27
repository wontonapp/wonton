use serde::{Deserialize, Serialize};

use super::{
    AskReceipt, BotInstallRow, ChatAccount, ChatContact, ChatProblem, ChatSync, EncryptionStatus,
    OwnBotProblem, OwnPrivateBotView, PendingConfirmation, RoomRow,
};
use crate::{BotView, ChatLayoutChoice, PushNotification, PushStatus};

/// 聊天页的 ViewModel。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ChatViewModel {
    pub account: ChatAccount,
    pub sync: ChatSync,
    pub encryption: EncryptionStatus,
    /// 有 Bot 的插件在装、在升级、装失败时的占位行，显示在 `rooms` 上面。装好、Bot 的私聊出现在 `rooms` 里以后
    /// 由那一行代替，这里不再有它。
    #[serde(default)]
    pub installing_bots: Vec<BotInstallRow>,
    /// 房间，按最近活动排序：最近有消息的在前，正在发送的在最前。
    pub rooms: Vec<RoomRow>,
    /// 新建聊天里的可选的人和 Bot（不能用的也留着，带 [`ChatContact::access`] 的原因）：
    /// 核心按当前权限把装着的 Bot、本人 Bot 和私聊过的人合成一表，能用的在前；壳只按搜索词过滤。
    #[serde(default)]
    pub shared_contacts: Vec<ChatContact>,
    /// 最近一次在后台失败的操作（加入房间、发起私聊、设置恢复等）；壳显示后用
    /// [`ChatAction::DismissChatProblem`](crate::ChatAction::DismissChatProblem) 清掉。
    pub problem: Option<ChatProblem>,
    /// 推送：App 被杀或锁屏时，新消息能不能弹出通知。
    pub push: PushStatus,
    /// 等主人回答的确认卡片，最早到期的在前。主人不在卡片所在的房间时，壳提示他去回答。
    pub pending_confirmations: Vec<PendingConfirmation>,
    /// 这台设备知道已经不用回答的卡片 ID：壳撤下它们的通知。
    pub settled_confirmations: Vec<String>,
    /// 收新消息的方式是 [`PushMethod::Keepalive`](crate::PushMethod::Keepalive) 且 App 在后台时，同步到的、按 Matrix
    /// 推送规则该通知的消息，先到的在前。壳逐条弹出系统通知，同一个 `event_id` 用同一个通知标签，重复给不会重复弹；
    /// 回到前台时核心清空。
    pub background_notifications: Vec<PushNotification>,
    /// 已装插件的 Bot：名字、头像、状态头像和此刻的状态（ADR-0044）。壳按 [`RoomRow::direct_user_id`] 找私聊对方是哪个 Bot。
    pub bots: Vec<BotView>,
    /// 本人私人 Bot 的身份事实；失败走 [`Self::own_private_bot_problem`]，不占全局 `problem`。
    /// 旧核心的流里没有这个键时逐字段 serde(default) 取 None，壳不能把缺键当成确认 Absent。
    #[serde(default)]
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub own_private_bot: Option<OwnPrivateBotView>,
    /// 读取本人私人 Bot 失败的问题。读取成功或恢复后由核心清掉。
    #[serde(default)]
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub own_private_bot_problem: Option<OwnBotProblem>,
    /// 最近一次“问 Bot”（[`ChatAction::AskAgent`](crate::ChatAction::AskAgent)）的回执：核心解析出哪个
    /// Bot、哪个房间，以及发没发出去。壳用 `request_id` 对上自己发起的那一次，按 `room_id` 打开私聊，
    /// 失败按 `problem.reason` 写人话并提供重试。还没问过、或者上一次已被
    /// [`ChatAction::DismissChatProblem`](crate::ChatAction::DismissChatProblem) 清掉时为空。
    #[serde(default)]
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub ask: Option<AskReceipt>,
    /// 聊天显示方式的全局默认里用户改过的项（ADR-0060），设置页的“聊天显示”读它。
    pub chat_layout_default: ChatLayoutChoice,
}
