use serde::{Deserialize, Serialize};

use crate::BotRef;

/// 插件的运行形态，独立于清单的 `kind`。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum PluginRuntime {
    #[default]
    Compose,
    RemoteMcp,
    Skills,
    #[serde(other)]
    Other,
}

/// 插件的种类（清单的 `kind`）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum PluginKind {
    App,
    Agent,
    Channel,
    Model,
    System,
    #[serde(other)]
    Other,
}

/// 插件的工具和技能对哪些 Bot 生效；全局范围包括以后新建的 Bot。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PluginScope {
    All,
    Bots {
        bots: Vec<BotRef>,
    },
    #[serde(other)]
    Other,
}

/// `PUT`：请求体 [`PluginScope`]，返回 [`PluginDetail`](super::PluginDetail)。只有主人能用。
pub fn plugin_scope_path(plugin_id: &str) -> String {
    format!("/_wonton/api/plugins/{plugin_id}/scope")
}
