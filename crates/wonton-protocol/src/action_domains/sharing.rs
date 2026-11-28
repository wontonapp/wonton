use serde::{Deserialize, Serialize};

use crate::ShareGrantUpdate;

/// 插件分享操作。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SharingAction {
    /// 新增授权，只有分享方主人能做；未声明好友识别能力的插件要确认风险。
    CreatePluginShare {
        grant: ShareGrantUpdate,
    },
    UpdatePluginShare {
        id: String,
        grant: ShareGrantUpdate,
    },
    /// 立即拒绝后续请求，同时断开这个授权不再允许的现有长连接。
    RevokePluginShare {
        id: String,
    },
    /// 重取一个来源或全部来源的分享；None 表示全部。
    RefreshSharedApps {
        owner_user_id: Option<String>,
    },
    /// 来源和插件一起定位，不能只按插件 ID 打开；失败放在该项的 ShareProblem。
    OpenSharedApp {
        owner_user_id: String,
        plugin_id: String,
    },
}
