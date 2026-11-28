use serde::{Deserialize, Serialize};

use super::BotRef;
use crate::{BotChatUnavailable, PluginIcon};

/// 主人新建一个共享 Bot。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct NewSharedBot {
    /// 声明了 `bots` 的 Agent 插件。
    pub plugin_id: String,
    /// 插件里的 Bot 名，同时决定 Matrix 用户名 `<插件 ID 最后一段>-<Bot 名>`。
    pub bot: String,
}

/// [`SHARED_BOTS_PATH`](super::SHARED_BOTS_PATH) 的响应：当前请求人看到的 Bot 和可用性。
///
/// 列表是“能发现谁、为什么不能用”，不是联系人同意书；发送资格仍按房间和 Bot 的现有规则。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct SharedBotsView {
    /// 整份列表可不可信。请求人的身份、成员或角色读不明时是 [`SharedBotsState::Unknown`]，
    /// 这时 `bots` 照常给能列出的行，除“Bot 记录根本不存在”的 [`BotChatUnavailable::BotMissing`] 外，
    /// 每行的 [`SharedBot::access`] 都是 [`BotAccess::Unknown`]。
    pub state: SharedBotsState,
    pub bots: Vec<SharedBot>,
}

/// [`SharedBotsView`] 的可信度。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum SharedBotsState {
    /// 请求人的身份和权限读得到，每行 `access` 按权限和联系人事实算过。
    #[default]
    Ready,
    /// 请求人的身份、成员或角色读不明：行还在，但可用性不知道。
    Unknown,
    #[serde(other)]
    Other,
}

/// 列表里的一个 Bot。`bot` 是精确的插件加 Bot 名，不拿插件的默认 Bot 冒充别的 Bot。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct SharedBot {
    pub bot: BotRef,
    /// Bot 的 Matrix 用户 ID；Bot 记录还没建出来时为空。
    pub user_id: Option<String>,
    /// 插件名，按请求人的语言挑过。
    pub title: String,
    /// 插件图标；插件没有图标时为空。
    pub icon: Option<PluginIcon>,
    pub access: BotAccess,
}

/// 当前请求人能不能明确用这个 Bot，以及为什么。
///
/// 和 [`crate::BotChatAvailability`] 的区别是它回答“凭什么”：本人的、共享给你的、已经是联系人的，
/// 哪怕都能聊，分组和文案也不一样。`Unusable` 是确定的不能用；[`BotAccess::Unknown`] 是还核实不了，
/// 发送照走离线队列。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum BotAccess {
    /// 这是自己的私人 Bot。
    Own,
    /// 家庭共享 Bot，自己的角色能听共享 Bot。
    Shared,
    /// 和它已经有成立的联系人。
    Contact,
    Unusable {
        reason: BotChatUnavailable,
    },
    #[default]
    #[serde(other)]
    Unknown,
}
