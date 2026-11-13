use serde::{Deserialize, Serialize};

use super::{PluginProblem, PluginState};

/// 一次由本设备发起的插件启动；不改变服务器报告的插件状态。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PluginStart {
    pub plugin_id: String,
    pub state: PluginStartState,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum PluginStartState {
    Starting,
    Failed { problem: PluginProblem },
}

/// Agent 私聊背后的插件未就绪时的提示；不影响 Matrix 房间的发送权限。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct RoomPlugin {
    pub plugin_id: String,
    pub status: PluginState,
    pub start: Option<PluginStartState>,
    /// 显示 `Action::StartPlugin`；启动请求进行中时为 false。
    pub can_start: bool,
}
