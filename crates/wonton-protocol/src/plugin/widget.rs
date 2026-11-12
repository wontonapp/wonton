//! 插件网页作为 Matrix widget 在房间里打开时要的能力（ADR-0096）：确认页和详情页按级别显示。
//!
//! 核心只给清单里的原始能力串和级别，文字由壳按级别写。

use serde::{Deserialize, Serialize};

/// widget 能力的级别：普通的不标色，高级的醒目显示。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum WidgetCapabilityLevel {
    /// 不碰聊天内容、成员和权限的事件：通话状态、给设备的消息、延迟发送这类。
    #[default]
    Normal,
    /// 会读到聊天内容，或者改历史、成员、权限的事件类型。
    High,
    /// 比这个 App 新的核心给的级别，按高级处理。
    #[serde(other)]
    Other,
}

/// 清单 `matrix_widget.capabilities` 的一项：原始能力串和它的级别。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct WidgetCapability {
    /// 清单里的写法，如 `org.matrix.msc2762.receive.state_event:org.matrix.msc3401.call.member`。
    pub capability: String,
    /// 级别；不认识的取值落到 [`WidgetCapabilityLevel::Other`]。
    #[serde(default)]
    pub level: WidgetCapabilityLevel,
    /// 升级时新申请的，标出来。新装时都是 `false`。
    #[serde(default)]
    pub added: bool,
}
