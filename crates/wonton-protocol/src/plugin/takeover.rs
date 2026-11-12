//! 这台设备上的“我来操作”。

use serde::{Deserialize, Serialize};

use super::PluginProblem;

/// 这台设备上的“我来操作”，同时只操作一个插件。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PluginTakeover {
    pub id: String,
    pub state: TakeoverState,
}

/// “我来操作”走到哪一步了。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum TakeoverState {
    /// 请求发出去了，家里的服务器还没回：Bot 的调用还没有停住。
    Starting,
    /// 家里的服务器确认了：Bot 的敏感调用都先停住。
    Operating,
    /// 没开始成，Bot 的调用照常判定。
    NotStarted { problem: PluginProblem },
    /// 操作到一半断了：续不上约，或者插件停了、设备被吊销。Bot 的调用不再停住。
    Lost { problem: PluginProblem },
}
