use serde::{Deserialize, Serialize};

use crate::{AvatarUpload, BotActivity};

/// Bot 资料操作。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BotAction {
    /// 改 Bot 的名字。名字先过 [`check_bot_name`](crate::check_bot_name)，不行时返回 `InvalidInput`。
    SetBotName { plugin_id: String, name: String },
    /// 换 Bot 的头像：`state` 为空是静态头像，否则是这个状态的头像。核心先以主人的身份传到 Matrix 的媒体库，再请家里的
    /// 服务器写进 Bot 的资料。
    SetBotAvatar {
        plugin_id: String,
        state: Option<BotActivity>,
        upload: AvatarUpload,
    },
    /// 清掉静态头像（`state` 为空）或一个状态的头像。
    ClearBotAvatar {
        plugin_id: String,
        state: Option<BotActivity>,
    },
    /// 清掉这个 Bot 的 `BotView::problem`。
    DismissBotProfileProblem { plugin_id: String },
    /// 头像换成云朵（ADR-0126），整套一起设：核心过 [`BotCloud::check`](crate::BotCloud::check)、合成、以 Bot 的身份
    /// 写进资料，并清掉主人传的那组图。不行时返回 `InvalidInput`。[`Self::SetBotAvatar`] 是另一个来源“自己的图”。
    SetBotCloud {
        plugin_id: String,
        cloud: crate::BotCloud,
    },
    /// 只改云朵的一项，其余两项不变。
    SetBotCloudPart {
        plugin_id: String,
        part: crate::CloudPart,
    },
}
