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
    /// 静态网页，文件交给提供能力 `static_web` 的插件托管（ADR-0080）。
    StaticWeb,
    /// 外部网页：只有一个地址，壳用系统的 App 内浏览器打开（ADR-0080）。
    Link,
    #[serde(other)]
    Other,
}

/// 核心按清单算出的安全档位（ADR-0080），作者不能填。和来源标签（[`super::PluginTrust`]）分开显示。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum SecurityTier {
    /// 外部网页，或不连外部地址的静态网页。
    Minimal,
    Standard,
    High,
    VeryHigh,
    Highest,
    /// 比这个 App 新的核心给的档位，按 [`Self::Highest`] 画。
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
