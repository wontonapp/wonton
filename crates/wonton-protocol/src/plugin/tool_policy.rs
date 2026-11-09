//! 主人放宽确认的设置页（ADR-0024）：`GET /_wonton/api/plugins/{id}/tool-policy` 和各个开关接口的响应。

use serde::{Deserialize, Serialize};

/// 一个插件的工具和放宽。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ToolPolicyView {
    pub tools: Vec<ToolPolicyEntry>,
    /// 列不出工具的原因（插件没运行等）；这时 `tools` 为空。
    pub tools_error: Option<String>,
    /// 各个 Bot 调这个插件是否始终允许。
    pub always: Vec<BotSwitch>,
    /// 这个插件自己的 Bot 调所有插件是否一律允许；插件没有 Bot 时为空。
    pub allow_all: Vec<BotSwitch>,
    /// 生效中的放行时段：到期前这些 Bot 调这个插件不发卡片。
    pub windows: Vec<OpenWindow>,
    /// 放宽的设置读不出来的原因：这时没有任何放宽，也改不了。
    pub problem: Option<String>,
}

/// 插件的一个工具。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ToolPolicyEntry {
    pub name: String,
    pub description: Option<String>,
    pub access: ToolAccess,
}

/// 调这个工具要不要问主人（放宽之前）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum ToolAccess {
    /// 要问；主人能标成只读。
    Sensitive,
    /// 插件自己标了只读，主人改不了。
    PluginReadOnly,
    /// 主人标了只读，能取消。
    OwnerReadOnly,
}

/// 一个 Bot 的开关。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct BotSwitch {
    /// Bot 所在的插件 ID，开关接口里的 `{caller}`。
    pub caller: String,
    pub name: String,
    pub enabled: bool,
}

/// 生效中的放行时段（ADR-0024）。停止用打开它的卡片：[`crate::window_path`]。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct OpenWindow {
    pub card: String,
    pub caller: String,
    pub name: String,
    pub expires_at_ms: u64,
}

/// 升级前后比较工具的结果（ADR-0024）：工具要等新版本跑起来才知道。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AddedTools {
    /// 新版本多出来的工具，可以为空。
    Compared { names: Vec<String> },
    /// 比不了的原因：旧版本或新版本列不出工具。
    NotCompared { reason: String },
}
